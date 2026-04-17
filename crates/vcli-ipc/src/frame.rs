//! Length-prefixed framing per spec §IPC → Wire format.
//!
//! `u32` big-endian length, then exactly that many UTF-8 JSON bytes.
//! `MAX_FRAME_LEN` (4 MiB) caps both write and read paths. Programs reference
//! images by `sha256:…` (Decision §Persistence → Asset store), so no binary
//! payload need cross the wire, and 4 MiB is generous for program JSON.

use serde::{de::DeserializeOwned, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::error::{IpcError, IpcResult};

/// Maximum single-frame payload. Enforced on read and write.
pub const MAX_FRAME_LEN: u32 = 4 * 1024 * 1024;

/// Serialize `value` to JSON and write a length-prefixed frame.
pub async fn write_frame<W, T>(w: &mut W, value: &T) -> IpcResult<()>
where
    W: AsyncWriteExt + Unpin,
    T: Serialize,
{
    let bytes = serde_json::to_vec(value)?;
    let len = u32::try_from(bytes.len())
        .map_err(|_| IpcError::FrameTooLarge { len: u32::MAX, max: MAX_FRAME_LEN })?;
    if len > MAX_FRAME_LEN {
        return Err(IpcError::FrameTooLarge { len, max: MAX_FRAME_LEN });
    }
    w.write_all(&len.to_be_bytes()).await?;
    w.write_all(&bytes).await?;
    w.flush().await?;
    Ok(())
}

/// Read one length-prefixed frame and deserialize it as `T`.
///
/// Returns `Err(IpcError::UnexpectedEof { got: 0, expected: 4 })` on clean EOF
/// before any bytes of the next header — callers use this to detect graceful
/// peer shutdown between frames.
pub async fn read_frame<R, T>(r: &mut R) -> IpcResult<T>
where
    R: AsyncReadExt + Unpin,
    T: DeserializeOwned,
{
    let mut hdr = [0u8; 4];
    let mut filled = 0;
    while filled < 4 {
        let n = r.read(&mut hdr[filled..]).await?;
        if n == 0 {
            return Err(IpcError::UnexpectedEof { got: filled, expected: 4 });
        }
        filled += n;
    }
    let len = u32::from_be_bytes(hdr);
    if len > MAX_FRAME_LEN {
        return Err(IpcError::FrameTooLarge { len, max: MAX_FRAME_LEN });
    }
    let len_usize = len as usize;
    let mut buf = vec![0u8; len_usize];
    let mut filled = 0;
    while filled < len_usize {
        let n = r.read(&mut buf[filled..]).await?;
        if n == 0 {
            return Err(IpcError::UnexpectedEof { got: filled, expected: len_usize });
        }
        filled += n;
    }
    Ok(serde_json::from_slice(&buf)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use tokio::io::duplex;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Msg {
        id: u32,
        text: String,
    }

    #[tokio::test]
    async fn roundtrip_simple_value() {
        let (mut a, mut b) = duplex(256);
        let sent = Msg { id: 7, text: "hi".into() };
        write_frame(&mut a, &sent).await.unwrap();
        let got: Msg = read_frame(&mut b).await.unwrap();
        assert_eq!(got, sent);
    }

    #[tokio::test]
    async fn roundtrip_two_frames_back_to_back() {
        let (mut a, mut b) = duplex(256);
        let m1 = Msg { id: 1, text: "one".into() };
        let m2 = Msg { id: 2, text: "two".into() };
        write_frame(&mut a, &m1).await.unwrap();
        write_frame(&mut a, &m2).await.unwrap();
        let got1: Msg = read_frame(&mut b).await.unwrap();
        let got2: Msg = read_frame(&mut b).await.unwrap();
        assert_eq!(got1, m1);
        assert_eq!(got2, m2);
    }

    #[tokio::test]
    async fn write_rejects_oversize_payload() {
        // Build a payload whose serialized length definitely exceeds MAX_FRAME_LEN.
        let big = "x".repeat((MAX_FRAME_LEN as usize) + 100);
        let (mut a, _b) = duplex(64);
        let err = write_frame(&mut a, &big).await.unwrap_err();
        assert!(matches!(err, IpcError::FrameTooLarge { .. }), "{err:?}");
    }
}
