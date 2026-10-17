@echo off
call "C:\RustDeskBuildTools\VC\Auxiliary\Build\vcvars64.bat"
if errorlevel 1 exit /b %errorlevel%
set "PATH=C:\Users\Administrator\AppData\Local\Temp\opencode\rustup\toolchains\1.90.0-x86_64-pc-windows-msvc\bin;%PATH%"
set "CARGO_HOME=C:\Users\Administrator\AppData\Local\Temp\opencode\cargo"
set "VCPKG_ROOT=C:\Users\Administrator\AppData\Local\Temp\opencode\vcpkg-agent"
set "LIBCLANG_PATH=C:\Users\Administrator\AppData\Local\Temp\opencode\libclang-agent\clang\native"
set "CARGO_BUILD_JOBS=1"
set "RUSTDESK_SOS="
cargo check --offline --locked --lib --features flutter
exit /b %errorlevel%
