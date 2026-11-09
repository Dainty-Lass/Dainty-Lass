//! The out-of-process frame protocol (DD5): length-prefixed JSON frames over the child's stdin and stdout.
//!
//! A frame is a 4-byte big-endian length, then that many bytes of one JSON message. The child speaks first
//! with [`Hello`] (protocol, API version, manifest); the host answers [`Welcome`] or refuses and closes.
//! After that either side may call the other: a call carries an id, its reply the same id. The parser is a
//! trust boundary (DX2): lengths are bounded before anything is allocated, unknown variants and fields are
//! refused, and a broken frame ends the connection.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::chat::{ChatAnswer, ChatQuery};
use crate::game::{GameAnswer, GameCall};
use crate::ids::ModuleId;
use crate::manifest::ModuleManifest;
use crate::module::{
    AdminCall, AdminResponse, CommandOutput, CommandRequest, Event, HostAnswer, HostCall, ModuleConfig, ModuleError,
    ModuleStatus,
};
use crate::network::NodeIdentity;
use crate::registrar::Registration;
use crate::version::{API_VERSION, ApiVersion};

/// The frame protocol's own version, checked in the handshake. A change to framing or to the message
/// envelopes raises it; the host refuses any other.
pub const PROTOCOL_VERSION: u32 = 1;

/// The largest frame either side accepts (1 MiB): a registration with every content file fits many times.
pub const MAX_FRAME: usize = 1 << 20;

/// The largest handshake frame: a manifest is small, and nothing bigger is read before it is checked.
pub const MAX_HELLO_FRAME: usize = 64 << 10;

const LENGTH_BYTES: usize = 4;

/// The child's first frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hello {
    /// The frame protocol it speaks.
    pub protocol: u32,
    /// The module API it was built against.
    pub api: ApiVersion,
    /// Its manifest.
    pub manifest: ModuleManifest,
}

impl Hello {
    /// The hello this build sends for `manifest`.
    pub fn new(manifest: ModuleManifest) -> Self {
        Self {
            protocol: PROTOCOL_VERSION,
            api: API_VERSION,
            manifest,
        }
    }
}

/// The host's answer to an accepted [`Hello`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Welcome {
    /// The frame protocol the host speaks.
    pub protocol: u32,
    /// The module API the host speaks.
    pub api: ApiVersion,
    /// The node the module runs on.
    pub node: NodeIdentity,
}

/// The host's calls into a module.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ModuleCall {
    /// What the module needs from the core plugins.
    Register,
    /// Start serving with this configuration.
    Start(ModuleConfig),
    /// Stop serving.
    Stop,
    /// The module's status.
    Status,
    /// Run a console command.
    Command(CommandRequest),
    /// Run an admin action.
    Admin(AdminCall),
    /// Handle an event the module subscribed to.
    Event(Event),
    /// Answer the chat plugin.
    Chat(ChatQuery),
    /// Answer a question about the game (MS1).
    Game(GameCall),
}

/// A module's answer to a [`ModuleCall`], the variant matching the call's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ModuleAnswer {
    /// What the module registered.
    Registered(Registration),
    /// The module started.
    Started,
    /// The module stopped.
    Stopped,
    /// The module's status.
    Status(ModuleStatus),
    /// The command's output.
    Command(CommandOutput),
    /// The admin action's result.
    Admin(AdminResponse),
    /// The event was handled.
    EventHandled,
    /// The chat answer.
    Chat(ChatAnswer),
    /// The game answer.
    Game(GameAnswer),
}

/// What the child writes to its stdout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case", deny_unknown_fields)]
pub enum ToHost {
    /// The handshake's first frame.
    Hello(Box<Hello>),
    /// A call into the host.
    Call {
        /// Chosen by the child; the reply carries it back.
        id: u64,
        /// The call.
        call: HostCall,
    },
    /// The answer to the host's call with the same id.
    Reply {
        /// The host call's id.
        id: u64,
        /// The module's answer or error.
        result: Result<ModuleAnswer, ModuleError>,
    },
}

/// What the host writes to the child's stdin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case", deny_unknown_fields)]
pub enum ToModule {
    /// The hello was accepted.
    Welcome(Welcome),
    /// The hello was refused; the host closes the connection after it.
    Refused {
        /// Why the host refused.
        reason: String,
    },
    /// A call into the module.
    Call {
        /// Chosen by the host; the reply carries it back.
        id: u64,
        /// The call.
        call: ModuleCall,
    },
    /// The answer to the module's call with the same id.
    Reply {
        /// The module call's id.
        id: u64,
        /// The host's answer or error.
        result: Result<HostAnswer, ModuleError>,
    },
}

/// Why a frame could not be read or written; any of them ends the connection.
#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    /// The length prefix is over the limit.
    #[error("a frame of {len} bytes is over the {max}-byte limit")]
    TooLarge {
        /// The frame's length in bytes.
        len: usize,
        /// The limit it broke.
        max: usize,
    },
    /// The length prefix is 0.
    #[error("an empty frame")]
    Empty,
    /// The stream ended partway through a frame.
    #[error("the stream ended inside a frame")]
    Truncated,
    /// The body is not a known message.
    #[error("a frame that is not a known message: {0}")]
    Malformed(#[from] serde_json::Error),
    /// Reading or writing the stream failed.
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

fn check_len(len: usize, max: usize) -> Result<(), FrameError> {
    if len == 0 {
        Err(FrameError::Empty)
    } else if len > max {
        Err(FrameError::TooLarge { len, max })
    } else {
        Ok(())
    }
}

/// One message as a frame, length included.
pub fn encode<T: Serialize>(message: &T, max: usize) -> Result<Vec<u8>, FrameError> {
    let body = serde_json::to_vec(message)?;
    check_len(body.len(), max)?;
    let len = u32::try_from(body.len()).map_err(|_| FrameError::TooLarge { len: body.len(), max })?;
    let mut frame = Vec::with_capacity(LENGTH_BYTES + body.len());
    frame.extend_from_slice(&len.to_be_bytes());
    frame.extend_from_slice(&body);
    Ok(frame)
}

/// The first message in `buf` and the bytes it took, or `None` when `buf` holds less than one frame. The
/// length is checked before the body is looked at.
pub fn decode<T: DeserializeOwned>(buf: &[u8], max: usize) -> Result<Option<(T, usize)>, FrameError> {
    let Some((head, rest)) = buf.split_first_chunk::<LENGTH_BYTES>() else {
        return Ok(None);
    };
    let len = u32::from_be_bytes(*head) as usize;
    check_len(len, max)?;
    let Some(body) = rest.get(..len) else {
        return Ok(None);
    };
    Ok(Some((serde_json::from_slice(body)?, LENGTH_BYTES + len)))
}

/// Reads one message; `None` when the stream ends cleanly between frames.
pub async fn read_frame<T: DeserializeOwned, R: AsyncRead + Unpin>(
    reader: &mut R,
    max: usize,
) -> Result<Option<T>, FrameError> {
    let mut head = [0u8; LENGTH_BYTES];
    let mut filled = 0;
    while filled < LENGTH_BYTES {
        match reader.read(&mut head[filled..]).await? {
            0 if filled == 0 => return Ok(None),
            0 => return Err(FrameError::Truncated),
            n => filled += n,
        }
    }
    let len = u32::from_be_bytes(head) as usize;
    check_len(len, max)?;
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).await.map_err(|error| match error.kind() {
        std::io::ErrorKind::UnexpectedEof => FrameError::Truncated,
        _ => FrameError::Io(error),
    })?;
    Ok(Some(serde_json::from_slice(&body)?))
}

/// Writes one message as a frame and flushes; a message over `max` is refused before anything is written.
pub async fn write_frame<T: Serialize, W: AsyncWrite + Unpin>(
    writer: &mut W,
    message: &T,
    max: usize,
) -> Result<(), FrameError> {
    let frame = encode(message, max)?;
    writer.write_all(&frame).await?;
    writer.flush().await?;
    Ok(())
}

/// Why the host refused a child's hello.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    /// The child speaks another frame protocol, the one held.
    #[error("frame protocol {0} is not {PROTOCOL_VERSION}")]
    Protocol(u32),
    /// The host's API does not accept the module's.
    #[error("module API {module} can't run on this host's {host}")]
    Api {
        /// The API the module was built against.
        module: ApiVersion,
        /// The host's API.
        host: ApiVersion,
    },
    /// The manifest is invalid; holds the reason.
    #[error("{0}")]
    Manifest(String),
    /// The executable is another module than the one `host.toml` named.
    #[error("expected module {expected}, the executable is {found}")]
    WrongModule {
        /// The module `host.toml` named.
        expected: ModuleId,
        /// The module the executable is.
        found: ModuleId,
    },
}

/// The host's check of a child's hello: the protocol and API it speaks, its manifest, and that it is the
/// module `host.toml` named for that executable.
pub fn check_hello(hello: &Hello, expected: Option<&ModuleId>) -> Result<(), Refusal> {
    if hello.protocol != PROTOCOL_VERSION {
        return Err(Refusal::Protocol(hello.protocol));
    }
    if !API_VERSION.accepts(hello.api) {
        return Err(Refusal::Api {
            module: hello.api,
            host: API_VERSION,
        });
    }
    if hello.manifest.api != hello.api {
        return Err(Refusal::Manifest(String::from(
            "the manifest's API version is not the hello's",
        )));
    }
    hello
        .manifest
        .validate()
        .map_err(|error| Refusal::Manifest(error.to_string()))?;
    if let Some(expected) = expected
        && expected != &hello.manifest.id
    {
        return Err(Refusal::WrongModule {
            expected: expected.clone(),
            found: hello.manifest.id.clone(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GameModule;
    use crate::stub::StubModule;

    fn hello() -> Hello {
        Hello::new(StubModule::new().manifest())
    }

    #[test]
    fn frames_round_trip_and_split_anywhere() {
        let messages = vec![
            ToHost::Hello(Box::new(hello())),
            ToHost::Call {
                id: 7,
                call: HostCall::Log(crate::LogRecord {
                    level: crate::LogLevel::Info,
                    message: "x".into(),
                }),
            },
            ToHost::Reply {
                id: 7,
                result: Err(ModuleError::unsupported("chat")),
            },
        ];
        let mut stream = Vec::new();
        for message in &messages {
            stream.extend(encode(message, MAX_FRAME).unwrap());
        }
        let mut decoded = Vec::new();
        let mut at = 0;
        while let Some((message, used)) = decode::<ToHost>(&stream[at..], MAX_FRAME).unwrap() {
            decoded.push(message);
            at += used;
        }
        assert_eq!(at, stream.len());
        assert_eq!(decoded, messages);
        for cut in 0..LENGTH_BYTES + 3 {
            assert!(decode::<ToHost>(&stream[..cut], MAX_FRAME).unwrap().is_none());
        }
    }

    #[test]
    fn bad_frames_are_refused_before_their_body_is_read() {
        assert!(matches!(
            decode::<ToHost>(&[0, 0, 0, 0], MAX_FRAME),
            Err(FrameError::Empty)
        ));
        assert!(matches!(
            decode::<ToHost>(&[0xff, 0xff, 0xff, 0xff], MAX_FRAME),
            Err(FrameError::TooLarge { len: 0xffff_ffff, .. })
        ));
        let small = encode(&ToHost::Hello(Box::new(hello())), MAX_FRAME).unwrap();
        assert!(matches!(decode::<ToHost>(&small, 16), Err(FrameError::TooLarge { .. })));
        assert!(matches!(
            encode(&ToHost::Hello(Box::new(hello())), 16),
            Err(FrameError::TooLarge { .. })
        ));
        for body in [
            &b"{}"[..],
            b"null",
            b"{\"t\":\"surprise\"}",
            b"{\"t\":\"call\",\"id\":1,\"call\":\"stop\"}",
        ] {
            let mut frame = (body.len() as u32).to_be_bytes().to_vec();
            frame.extend_from_slice(body);
            assert!(
                matches!(decode::<ToHost>(&frame, MAX_FRAME), Err(FrameError::Malformed(_))),
                "{body:?}"
            );
        }
        let extra = br#"{"t":"reply","id":1,"result":{"Ok":"started"},"x":1}"#;
        let mut frame = (extra.len() as u32).to_be_bytes().to_vec();
        frame.extend_from_slice(extra);
        assert!(decode::<ToHost>(&frame, MAX_FRAME).is_err());
    }

    #[tokio::test]
    async fn streams_end_cleanly_only_between_frames() {
        let frame = encode(&ToModule::Refused { reason: "no".into() }, MAX_FRAME).unwrap();
        let mut whole = &frame[..];
        assert!(
            read_frame::<ToModule, _>(&mut whole, MAX_FRAME)
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            read_frame::<ToModule, _>(&mut whole, MAX_FRAME)
                .await
                .unwrap()
                .is_none()
        );
        for cut in [2, LENGTH_BYTES + 1, frame.len() - 1] {
            let mut part = &frame[..cut];
            assert!(matches!(
                read_frame::<ToModule, _>(&mut part, MAX_FRAME).await,
                Err(FrameError::Truncated)
            ));
        }
        let mut huge = &[0x7f, 0, 0, 0][..];
        assert!(matches!(
            read_frame::<ToModule, _>(&mut huge, MAX_FRAME).await,
            Err(FrameError::TooLarge { .. })
        ));
    }

    #[test]
    fn the_handshake_refuses_other_versions_and_modules() {
        let good = hello();
        assert_eq!(check_hello(&good, None), Ok(()));
        assert_eq!(check_hello(&good, Some(&good.manifest.id)), Ok(()));
        let other: ModuleId = "isles-1-14-7".parse().unwrap();
        assert!(matches!(
            check_hello(&good, Some(&other)),
            Err(Refusal::WrongModule { .. })
        ));
        let mut old = hello();
        old.protocol = 0;
        assert_eq!(check_hello(&old, None), Err(Refusal::Protocol(0)));
        let mut newer = hello();
        newer.api.minor += 1;
        newer.manifest.api = newer.api;
        assert!(matches!(check_hello(&newer, None), Err(Refusal::Api { .. })));
        let mut mismatch = hello();
        mismatch.manifest.api.minor += 1;
        assert!(matches!(check_hello(&mismatch, None), Err(Refusal::Manifest(_))));
        let mut bad = hello();
        bad.manifest.display_name.clear();
        assert!(matches!(check_hello(&bad, None), Err(Refusal::Manifest(_))));
    }
}
