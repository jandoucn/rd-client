use super::{
    handoff::ObservedProcess,
    health::{Request, Response},
};
use hbb_common::tokio::{
    self,
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
};
use parity_tokio_ipc::{Endpoint, Incoming, SecurityAttributes};
use std::{
    os::windows::io::AsRawHandle,
    path::{Path, PathBuf},
    time::Duration,
};

pub fn bind(path: &str) -> Result<Incoming, String> {
    if !path.starts_with(r"\\.\pipe\RustDesk") {
        return Err("health pipe path invalid".into());
    }
    let mut endpoint = Endpoint::new(path.into());
    endpoint.set_security_attributes(
        SecurityAttributes::from_sddl("D:P(A;;GA;;;SY)(A;;GA;;;BA)").map_err(|e| e.to_string())?,
    );
    endpoint
        .incoming()
        .map_err(|e| format!("health listener: {e}"))
}

async fn receive(stream: &mut (impl AsyncRead + Unpin)) -> Result<Vec<u8>, String> {
    let count = stream.read_u32_le().await.map_err(|e| e.to_string())?;
    if count == 0 || count > 4096 {
        return Err("health packet length invalid".into());
    }
    let mut bytes = vec![0; count as usize];
    stream
        .read_exact(&mut bytes)
        .await
        .map_err(|e| e.to_string())?;
    Ok(bytes)
}

async fn send(stream: &mut (impl AsyncWrite + Unpin), bytes: &[u8]) -> Result<(), String> {
    if bytes.is_empty() || bytes.len() > 4096 {
        return Err("health response length invalid".into());
    }
    stream
        .write_u32_le(bytes.len() as u32)
        .await
        .map_err(|e| e.to_string())?;
    stream.write_all(bytes).await.map_err(|e| e.to_string())?;
    stream.flush().await.map_err(|e| e.to_string())
}

pub async fn serve(
    mut listener: Incoming,
    version: String,
    build_seq: u64,
    registered: impl Fn() -> bool + Send + 'static,
) -> Result<(), String> {
    let owner = {
        let process = ObservedProcess::open(std::process::id())?;
        process.owner().clone()
    };
    while let Some(connection) = listener.next().await {
        let mut stream = connection.map_err(|e| e.to_string())?;
        let result = tokio::time::timeout(Duration::from_secs(2), async {
            let request = Request::decode(&receive(&mut stream).await?)?;
            let response = Response::new(
                &request,
                owner.pid,
                owner.created_at,
                version.clone(),
                build_seq,
                true,
                registered(),
            )?;
            send(
                &mut stream,
                &serde_json::to_vec(&response).map_err(|e| e.to_string())?,
            )
            .await
        })
        .await;
        if let Ok(Err(error)) = result {
            hbb_common::log::trace!("local health request refused: {error}");
        }
    }
    Err("health listener ended".into())
}

pub async fn serve_activity(
    mut listener: Incoming,
    version: String,
    build_seq: u64,
    registered: impl Fn() -> bool + Send + 'static,
    idle: impl Fn() -> bool + Send + 'static,
) -> Result<(), String> {
    let owner = {
        let process = ObservedProcess::open(std::process::id())?;
        process.owner().clone()
    };
    while let Some(connection) = listener.next().await {
        let mut stream = connection.map_err(|error| error.to_string())?;
        let result = tokio::time::timeout(Duration::from_secs(2), async {
            let request = Request::decode(&receive(&mut stream).await?)?;
            let mut response = Response::new(&request, owner.pid, owner.created_at,
                version.clone(), build_seq, true, registered())?;
            response.no_active_sessions = Some(idle());
            send(&mut stream, &serde_json::to_vec(&response).map_err(|error| error.to_string())?).await
        }).await;
        if let Ok(Err(error)) = result {
            hbb_common::log::trace!("local activity request refused: {error}");
        }
    }
    Err("activity health listener ended".into())
}

fn server_executable(handle: *mut std::ffi::c_void) -> Result<(ObservedProcess, PathBuf), String> {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetNamedPipeServerProcessId(pipe: *mut std::ffi::c_void, pid: *mut u32) -> i32;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
        fn QueryFullProcessImageNameW(
            process: *mut std::ffi::c_void,
            flags: u32,
            name: *mut u16,
            count: *mut u32,
        ) -> i32;
        fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
    }
    let mut pid = 0;
    if unsafe { GetNamedPipeServerProcessId(handle, &mut pid) } == 0 {
        return Err(format!(
            "health pipe server identity: {}",
            std::io::Error::last_os_error()
        ));
    }
    let process = ObservedProcess::open(pid)?;
    let queried = unsafe { OpenProcess(0x1000, 0, pid) };
    if queried.is_null() {
        return Err("health server process inaccessible".into());
    }
    let mut buffer = vec![0u16; 32768];
    let mut count = buffer.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(queried, 0, buffer.as_mut_ptr(), &mut count) };
    let error = std::io::Error::last_os_error();
    if unsafe { CloseHandle(queried) } == 0 {
        return Err(format!(
            "health process handle cleanup: {}",
            std::io::Error::last_os_error()
        ));
    }
    if ok == 0 {
        return Err(format!("health server executable query: {error}"));
    }
    let path = PathBuf::from(
        String::from_utf16(&buffer[..count as usize])
            .map_err(|_| "health executable encoding invalid".to_owned())?,
    );
    Ok((process, path))
}

pub async fn probe(
    path: &str,
    expected_executable: &Path,
    build_seq: u64,
    timeout: Duration,
) -> Result<Response, String> {
    tokio::time::timeout(timeout, async {
        let mut stream = Endpoint::connect(path)
            .await
            .map_err(|e| format!("health connect: {e}"))?;
        let (process, path) = server_executable(stream.as_raw_handle())?;
        let observed = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
        let expected = std::fs::canonicalize(expected_executable)
            .map_err(|e| format!("health expected executable unavailable: {e}"))?;
        if !observed
            .to_string_lossy()
            .eq_ignore_ascii_case(&expected.to_string_lossy())
        {
            return Err("health server executable mismatch".into());
        }
        if !process.is_alive()? {
            return Err("health server process exited".into());
        }
        let request = Request::new(hex::encode(
            hbb_common::sodiumoxide::randombytes::randombytes(32),
        ))?;
        send(
            &mut stream,
            &serde_json::to_vec(&request).map_err(|e| e.to_string())?,
        )
        .await?;
        let response = Response::decode(&receive(&mut stream).await?)?;
        response.verify(
            &request,
            process.owner().pid,
            process.owner().created_at,
            build_seq,
        )?;
        if !process.is_alive()? {
            return Err("health server process changed/exited".into());
        }
        Ok(response)
    })
    .await
    .map_err(|_| "health probe timeout".to_owned())?
}

#[cfg(windows)]
pub fn probe_blocking(
    path: &str,
    expected_executable: &Path,
    build_seq: u64,
    timeout: Duration,
) -> Result<Response, String> {
    use std::{
        io::{Read, Write},
        os::windows::fs::OpenOptionsExt,
        time::Instant,
    };
    #[link(name = "kernel32")]
    extern "system" {
        fn SetNamedPipeHandleState(
            pipe: *mut std::ffi::c_void,
            mode: *const u32,
            max: *const u32,
            timeout: *const u32,
        ) -> i32;
    }
    let deadline = Instant::now() + timeout;
    let mut stream = loop {
        match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(3)
            .open(path)
        {
            Ok(file) => break file,
            Err(error) if error.raw_os_error() == Some(231) && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(error) => return Err(format!("health blocking connect: {error}")),
        }
    };
    let mode = 1u32;
    if unsafe {
        SetNamedPipeHandleState(
            stream.as_raw_handle(),
            &mode,
            std::ptr::null(),
            std::ptr::null(),
        )
    } == 0
    {
        return Err(format!(
            "health nonblocking pipe: {}",
            std::io::Error::last_os_error()
        ));
    }
    let (process, server) = server_executable(stream.as_raw_handle())?;
    let observed = std::fs::canonicalize(server).map_err(|e| e.to_string())?;
    let expected = std::fs::canonicalize(expected_executable).map_err(|e| e.to_string())?;
    if !observed
        .to_string_lossy()
        .eq_ignore_ascii_case(&expected.to_string_lossy())
    {
        return Err("health server executable mismatch".into());
    }
    let request = Request::new(hex::encode(
        hbb_common::sodiumoxide::randombytes::randombytes(32),
    ))?;
    let body = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
    let mut packet = (body.len() as u32).to_le_bytes().to_vec();
    packet.extend_from_slice(&body);
    let mut written = 0;
    while written < packet.len() {
        if Instant::now() >= deadline {
            return Err("health blocking write timeout".into());
        }
        match stream.write(&packet[written..]) {
            Ok(0) => std::thread::sleep(Duration::from_millis(10)),
            Ok(count) => written += count,
            Err(error) if error.raw_os_error() == Some(232) => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(error) => return Err(format!("health blocking write: {error}")),
        }
    }
    let mut read_exact = |buffer: &mut [u8]| -> Result<(), String> {
        #[link(name="kernel32")]
        extern "system" {fn PeekNamedPipe(pipe:*mut std::ffi::c_void,buffer:*mut std::ffi::c_void,size:u32,read:*mut u32,available:*mut u32,left:*mut u32)->i32;}
        let mut read = 0;
        while read < buffer.len() {
            if Instant::now() >= deadline {
                return Err("health blocking read timeout".into());
            }
            let mut available=0u32;
            if unsafe{PeekNamedPipe(stream.as_raw_handle(),std::ptr::null_mut(),0,std::ptr::null_mut(),&mut available,std::ptr::null_mut())}==0{return Err(format!("health blocking peer unavailable: {}",std::io::Error::last_os_error()));}
            if available==0 {std::thread::sleep(Duration::from_millis(10));continue;}
            match stream.read(&mut buffer[read..]) {
                Ok(0) => return Err("health blocking peer closed".into()),
                Ok(count) => read += count,
                Err(error) if error.raw_os_error() == Some(232) => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(error) => return Err(format!("health blocking read: {error}")),
            }
        }
        Ok(())
    };
    let mut header = [0u8; 4];
    read_exact(&mut header)?;
    let count = u32::from_le_bytes(header);
    if count == 0 || count > 4096 {
        return Err("health packet length invalid".into());
    }
    let mut bytes = vec![0; count as usize];
    read_exact(&mut bytes)?;
    let response = Response::decode(&bytes)?;
    response.verify(
        &request,
        process.owner().pid,
        process.owner().created_at,
        build_seq,
    )?;
    if !process.is_alive()? {
        return Err("health server process exited".into());
    }
    Ok(response)
}
