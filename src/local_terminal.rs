//! 本机 SYSTEM 终端的线上请求格式。
//!
//! 用户进程（界面）经 `_terminal` IPC 把 `LocalRequest` 以 JSON 发给服务进程，服务进程把它
//! 转成 `TerminalAction` 交给 `TerminalServiceProxy`。这条通道对任意本机进程可达，因此
//! `decode` 是信任边界：解不出、不认识、或字段越界的请求一律返回 `None`，由服务端丢弃。

use base::message_proto::TerminalAction;

/// `_terminal` 通道的后缀。Windows 上它决定命名管道名，并参与对端可执行文件比对。
pub const POSTFIX: &str = "_terminal";

/// 单条输入的字节上限。界面一次粘贴更多内容时应自行分片。
pub const MAX_INPUT_BYTES: usize = 64 * 1024;
/// 行列上限。真实终端远小于此，超出视为畸形请求。
pub const MAX_DIMENSION: u32 = 1000;
/// 终端号上限。每条连接是独立的终端服务，号只在连接内有意义。
pub const MAX_TERMINAL_ID: i32 = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalRequest {
    Open {
        terminal_id: i32,
        rows: u32,
        cols: u32,
    },
    Input {
        terminal_id: i32,
        data: Vec<u8>,
    },
    Resize {
        terminal_id: i32,
        rows: u32,
        cols: u32,
    },
    Close {
        terminal_id: i32,
    },
}

/// 线上格式与内部枚举分开：线上用 serde 的外部标签 JSON 且拒绝未知字段，内部枚举不带 serde。
mod wire {
    use super::LocalRequest;
    use serde_derive::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct Open {
        pub terminal_id: i32,
        pub rows: u32,
        pub cols: u32,
    }

    #[derive(Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct Input {
        pub terminal_id: i32,
        pub data: Vec<u8>,
    }

    #[derive(Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct Close {
        pub terminal_id: i32,
    }

    #[derive(Serialize, Deserialize)]
    pub enum Wire {
        Open(Open),
        Input(Input),
        Resize(Open),
        Close(Close),
    }

    impl From<&LocalRequest> for Wire {
        fn from(request: &LocalRequest) -> Self {
            match request.clone() {
                LocalRequest::Open {
                    terminal_id,
                    rows,
                    cols,
                } => Wire::Open(Open {
                    terminal_id,
                    rows,
                    cols,
                }),
                LocalRequest::Input { terminal_id, data } => {
                    Wire::Input(Input { terminal_id, data })
                }
                LocalRequest::Resize {
                    terminal_id,
                    rows,
                    cols,
                } => Wire::Resize(Open {
                    terminal_id,
                    rows,
                    cols,
                }),
                LocalRequest::Close { terminal_id } => Wire::Close(Close { terminal_id }),
            }
        }
    }

    impl From<Wire> for LocalRequest {
        fn from(wire: Wire) -> Self {
            match wire {
                Wire::Open(o) => LocalRequest::Open {
                    terminal_id: o.terminal_id,
                    rows: o.rows,
                    cols: o.cols,
                },
                Wire::Input(i) => LocalRequest::Input {
                    terminal_id: i.terminal_id,
                    data: i.data,
                },
                Wire::Resize(o) => LocalRequest::Resize {
                    terminal_id: o.terminal_id,
                    rows: o.rows,
                    cols: o.cols,
                },
                Wire::Close(c) => LocalRequest::Close {
                    terminal_id: c.terminal_id,
                },
            }
        }
    }
}

impl LocalRequest {
    /// 编码前也做同样的校验：界面不该发出服务端一定会丢的请求。
    pub fn encode(&self) -> Option<Vec<u8>> {
        if !self.is_valid() {
            return None;
        }
        serde_json::to_vec(&wire::Wire::from(self)).ok()
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let request: Self = serde_json::from_slice::<wire::Wire>(bytes).ok()?.into();
        request.is_valid().then_some(request)
    }

    pub fn terminal_id(&self) -> i32 {
        match self {
            Self::Open { terminal_id, .. }
            | Self::Input { terminal_id, .. }
            | Self::Resize { terminal_id, .. }
            | Self::Close { terminal_id } => *terminal_id,
        }
    }

    fn is_valid(&self) -> bool {
        let id_ok = (0..=MAX_TERMINAL_ID).contains(&self.terminal_id());
        let dims_ok = |rows: u32, cols: u32| {
            (1..=MAX_DIMENSION).contains(&rows) && (1..=MAX_DIMENSION).contains(&cols)
        };
        id_ok
            && match self {
                Self::Open { rows, cols, .. } | Self::Resize { rows, cols, .. } => {
                    dims_ok(*rows, *cols)
                }
                Self::Input { data, .. } => data.len() <= MAX_INPUT_BYTES,
                Self::Close { .. } => true,
            }
    }

    pub fn to_action(&self) -> TerminalAction {
        use base::message_proto::{CloseTerminal, OpenTerminal, ResizeTerminal, TerminalData};
        let mut action = TerminalAction::new();
        match self.clone() {
            Self::Open {
                terminal_id,
                rows,
                cols,
            } => action.set_open(OpenTerminal {
                terminal_id,
                rows,
                cols,
                ..Default::default()
            }),
            Self::Input { terminal_id, data } => action.set_data(TerminalData {
                terminal_id,
                data: data.into(),
                compressed: false,
                ..Default::default()
            }),
            Self::Resize {
                terminal_id,
                rows,
                cols,
            } => action.set_resize(ResizeTerminal {
                terminal_id,
                rows,
                cols,
                ..Default::default()
            }),
            Self::Close { terminal_id } => action.set_close(CloseTerminal {
                terminal_id,
                ..Default::default()
            }),
        }
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base::message_proto::terminal_action::Union;

    fn all_valid() -> Vec<LocalRequest> {
        vec![
            LocalRequest::Open {
                terminal_id: 0,
                rows: 24,
                cols: 80,
            },
            LocalRequest::Input {
                terminal_id: 1,
                data: "dir\r中文".as_bytes().to_vec(),
            },
            LocalRequest::Resize {
                terminal_id: 1,
                rows: 45,
                cols: 140,
            },
            LocalRequest::Close { terminal_id: 1 },
        ]
    }

    #[test]
    fn every_request_round_trips() {
        for request in all_valid() {
            let bytes = request.encode().expect("a valid request must encode");
            assert_eq!(LocalRequest::decode(&bytes), Some(request));
        }
    }

    #[test]
    fn the_wire_format_is_externally_tagged_json() {
        let bytes = LocalRequest::Close { terminal_id: 2 }.encode().unwrap();
        assert_eq!(bytes, br#"{"Close":{"terminal_id":2}}"#.to_vec());
        assert_eq!(
            LocalRequest::decode(br#"{"Open":{"terminal_id":3,"rows":24,"cols":80}}"#),
            Some(LocalRequest::Open {
                terminal_id: 3,
                rows: 24,
                cols: 80
            })
        );
    }

    #[test]
    fn garbage_and_unknown_actions_are_rejected() {
        for bytes in [
            b"".to_vec(),
            b"not json".to_vec(),
            br#"{"Unknown":{"terminal_id":1}}"#.to_vec(),
            br#"{"Open":{"terminal_id":1}}"#.to_vec(),
            br#"{"Close":{"terminal_id":1,"extra":true}}"#.to_vec(),
            br#"{"Close":{"terminal_id":"1"}}"#.to_vec(),
            br#"[]"#.to_vec(),
        ] {
            assert_eq!(
                LocalRequest::decode(&bytes),
                None,
                "{:?}",
                String::from_utf8_lossy(&bytes)
            );
        }
    }

    #[test]
    fn out_of_range_fields_are_rejected_on_both_ends() {
        let bad = [
            LocalRequest::Open {
                terminal_id: -1,
                rows: 24,
                cols: 80,
            },
            LocalRequest::Open {
                terminal_id: MAX_TERMINAL_ID + 1,
                rows: 24,
                cols: 80,
            },
            LocalRequest::Open {
                terminal_id: 0,
                rows: 0,
                cols: 80,
            },
            LocalRequest::Open {
                terminal_id: 0,
                rows: 24,
                cols: MAX_DIMENSION + 1,
            },
            LocalRequest::Resize {
                terminal_id: 0,
                rows: 24,
                cols: 0,
            },
            LocalRequest::Input {
                terminal_id: 0,
                data: vec![b'a'; MAX_INPUT_BYTES + 1],
            },
            LocalRequest::Close { terminal_id: -5 },
        ];
        for request in bad {
            assert_eq!(request.encode(), None, "client must not send {request:?}");
            let raw = serde_json::to_vec(&wire::Wire::from(&request)).unwrap();
            assert_eq!(
                LocalRequest::decode(&raw),
                None,
                "service must drop {request:?}"
            );
        }
        // 边界值本身合法。
        let edge = LocalRequest::Open {
            terminal_id: MAX_TERMINAL_ID,
            rows: MAX_DIMENSION,
            cols: 1,
        };
        assert!(edge.encode().is_some());
        let input = LocalRequest::Input {
            terminal_id: 0,
            data: vec![0; MAX_INPUT_BYTES],
        };
        assert!(input.encode().is_some());
    }

    #[test]
    fn empty_input_is_allowed() {
        let request = LocalRequest::Input {
            terminal_id: 0,
            data: vec![],
        };
        assert_eq!(
            LocalRequest::decode(&request.encode().unwrap()),
            Some(request)
        );
    }

    #[test]
    fn requests_map_onto_the_matching_terminal_action() {
        match (LocalRequest::Open {
            terminal_id: 3,
            rows: 24,
            cols: 80,
        })
        .to_action()
        .union
        {
            Some(Union::Open(o)) => assert_eq!((o.terminal_id, o.rows, o.cols), (3, 24, 80)),
            other => panic!("{other:?}"),
        }
        match (LocalRequest::Input {
            terminal_id: 4,
            data: b"ls\r".to_vec(),
        })
        .to_action()
        .union
        {
            Some(Union::Data(d)) => {
                assert_eq!(d.terminal_id, 4);
                assert_eq!(d.data.to_vec(), b"ls\r".to_vec());
                assert!(
                    !d.compressed,
                    "raw input must never be flagged as compressed"
                );
            }
            other => panic!("{other:?}"),
        }
        match (LocalRequest::Resize {
            terminal_id: 5,
            rows: 45,
            cols: 140,
        })
        .to_action()
        .union
        {
            Some(Union::Resize(r)) => assert_eq!((r.terminal_id, r.rows, r.cols), (5, 45, 140)),
            other => panic!("{other:?}"),
        }
        match (LocalRequest::Close { terminal_id: 6 }).to_action().union {
            Some(Union::Close(c)) => assert_eq!(c.terminal_id, 6),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn postfix_is_the_dedicated_terminal_channel() {
        assert_eq!(POSTFIX, "_terminal");
        assert_ne!(POSTFIX, crate::POSTFIX_SERVICE);
    }
}
