#[allow(dead_code)]
pub mod broker;
#[allow(dead_code)]
pub mod owner;
use core::{
    future::Future,
    pin::pin,
    task::{Context, Poll},
};
use pajoniiir_core::MediaTrackId;
use pajoniiir_media_catalog::{CatalogBuilder, CatalogSnapshot, CatalogText, CatalogTrack};
use pajoniiir_media_fs::*;
use pajoniiir_media_session::{MediaHandle, MediaLease, MediaSession, MediaSourceId};
pub fn run<F: Future>(f: F) -> F::Output {
    match pin!(f)
        .as_mut()
        .poll(&mut Context::from_waker(core::task::Waker::noop()))
    {
        Poll::Ready(v) => v,
        Poll::Pending => panic!("unexpected pending"),
    }
}
// Test-only polling of self-waking fixtures, with a finite runaway guard.
#[allow(dead_code)]
pub fn drive<F: Future>(f: F) -> F::Output {
    let mut f = pin!(f);
    let mut cx = Context::from_waker(core::task::Waker::noop());
    for _ in 0..100_000 {
        if let Poll::Ready(value) = f.as_mut().poll(&mut cx) {
            return value;
        }
    }
    panic!("fixture exceeded poll budget")
}
pub fn mounted() -> MediaSession {
    let mut m = MediaSession::new();
    m.on_connect(MediaSourceId::new(1).unwrap());
    let l = m.lease().unwrap();
    assert!(m.bind_handle(l, MediaHandle::new(1).unwrap()));
    assert!(m.commit_mounted(l));
    m
}
pub fn catalog(m: &MediaSession) -> CatalogSnapshot<1> {
    let mut c = CatalogBuilder::new(m.lease().unwrap());
    c.push(CatalogTrack {
        track_id: MediaTrackId([7; 16]),
        source_track_id: 1,
        bpm_x100: 0,
        duration_ms: 0,
        title: CatalogText::EMPTY,
        artist: CatalogText::EMPTY,
        album: CatalogText::EMPTY,
        key: CatalogText::EMPTY,
        audio_path: CatalogText::try_from_str("/TRACK.WAV").unwrap(),
        analysis_path: CatalogText::EMPTY,
    })
    .unwrap();
    c.finish(1)
}
pub fn wav(frames: usize) -> Vec<u8> {
    let len = frames * 4;
    let mut b = Vec::new();
    b.extend(b"RIFF");
    b.extend(((36 + len) as u32).to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(3u16.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(8000u32.to_le_bytes());
    b.extend(32000u32.to_le_bytes());
    b.extend(4u16.to_le_bytes());
    b.extend(32u16.to_le_bytes());
    b.extend(b"data");
    b.extend((len as u32).to_le_bytes());
    for i in 0..frames {
        let phase = i % 4000;
        let v = if phase < 64 {
            (64 - phase) as f32 / 64.0 * if i / 4000 % 4 == 0 { 0.75 } else { 0.2 }
        } else {
            0.
        };
        b.extend(v.to_le_bytes());
    }
    b.extend(b"trailing identity bytes");
    b
}
#[derive(Debug, PartialEq)]
pub enum Fault {
    Io,
}
pub struct Handle {
    pub lease: MediaLease,
    pub len: u64,
    pub pos: u64,
}
impl LeaseBound for Handle {
    fn lease(&self) -> MediaLease {
        self.lease
    }
}
impl FileHandle for Handle {
    fn len(&self) -> u64 {
        self.len
    }
    fn position(&self) -> u64 {
        self.pos
    }
    fn set_position(&mut self, p: u64) {
        self.pos = p;
    }
}
impl DirectoryHandle for Handle {}
pub struct Fs {
    pub bytes: Vec<u8>,
    pub lease: MediaLease,
    pub max: usize,
    pub mode: u8,
    pub closes: usize,
    pub reads: usize,
}
impl Fs {
    pub fn new(m: &MediaSession, bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            lease: m.lease().unwrap(),
            max: 137,
            mode: 0,
            closes: 0,
            reads: 0,
        }
    }
}
impl LeaseBound for Fs {
    fn lease(&self) -> MediaLease {
        self.lease
    }
}
impl AsyncFileSystem for Fs {
    type Error = Fault;
    type File = Handle;
    type Directory = Handle;
    fn kind(&self) -> FileSystemKind {
        FileSystemKind::Fat32
    }
    fn capabilities(&self) -> FileSystemCapabilities {
        FileSystemCapabilities::READ_ONLY
    }
    async fn stat(&mut self, _: &str) -> Result<FileStat, Fault> {
        Err(Fault::Io)
    }
    async fn open_read(&mut self, path: &str) -> Result<Handle, Fault> {
        assert_eq!(path, "/TRACK.WAV");
        if self.mode == 11 {
            self.lease.source = MediaSourceId::new(2).unwrap();
        }
        Ok(Handle {
            lease: self.lease,
            len: self.bytes.len() as u64,
            pos: 0,
        })
    }
    async fn close_file(&mut self, _: Handle) -> Result<(), Fault> {
        self.closes += 1;
        if self.mode == 13 {
            self.lease.source = MediaSourceId::new(2).unwrap();
        }
        if self.mode == 10 {
            let mut first = true;
            core::future::poll_fn(|cx| {
                if first {
                    first = false;
                    cx.waker().wake_by_ref();
                    Poll::Pending
                } else {
                    Poll::Ready(())
                }
            })
            .await;
        }
        if self.mode == 5 {
            Err(Fault::Io)
        } else {
            Ok(())
        }
    }
    async fn read(&mut self, f: &mut Handle, out: &mut [u8]) -> Result<usize, Fault> {
        assert!(out.len() <= 4096);
        self.reads += 1;
        match self.mode {
            1 => return Err(Fault::Io),
            2 => return Ok(0),
            3 => return Ok(out.len() + 1),
            4 => core::future::pending::<()>().await,
            _ => (),
        }
        if self.mode == 8 {
            let mut first = true;
            core::future::poll_fn(|cx| {
                if first {
                    first = false;
                    cx.waker().wake_by_ref();
                    Poll::Pending
                } else {
                    Poll::Ready(())
                }
            })
            .await;
        }
        let n = out
            .len()
            .min(self.max)
            .min(self.bytes.len() - f.pos as usize);
        out[..n].copy_from_slice(&self.bytes[f.pos as usize..f.pos as usize + n]);
        f.pos += n as u64;
        if self.mode == 12 {
            self.lease.source = MediaSourceId::new(2).unwrap();
            f.lease = self.lease;
        }
        if self.mode == 6 {
            f.pos += 1;
        }
        if self.mode == 7 {
            f.len += 1;
        }
        if self.mode == 9 {
            core::future::pending::<()>().await;
        }
        Ok(n)
    }
    async fn seek_absolute(&mut self, _: &mut Handle, _: u64) -> Result<(), Fault> {
        Err(Fault::Io)
    }
    async fn open_directory(&mut self, _: &str) -> Result<Handle, Fault> {
        Err(Fault::Io)
    }
    async fn next_entry(
        &mut self,
        _: &mut Handle,
        _: &mut [u8; FS_NAME_MAX],
    ) -> Result<Option<DirEntry>, Fault> {
        Err(Fault::Io)
    }
    async fn close_directory(&mut self, _: Handle) -> Result<(), Fault> {
        Err(Fault::Io)
    }
}

pub struct Bank(pub Vec<CatalogTrack>);
impl pajoniiir_media_catalog::CatalogBank for Bank {
    fn capacity(&self) -> usize {
        1
    }
    fn tracks(&self) -> &[CatalogTrack] {
        &self.0
    }
    fn clear(&mut self) {
        self.0.clear();
    }
    fn push_prechecked(&mut self, track: CatalogTrack) -> usize {
        let i = self.0.len();
        self.0.push(track);
        i
    }
}

// Deterministic impulse PCM, odd padding, reordered extensible framing and trailer.
pub fn wav_profile(
    frames: usize,
    bits: u16,
    channels: u16,
    float: bool,
    extensible: bool,
) -> Vec<u8> {
    let original = wav(frames);
    let mut pcm = Vec::new();
    for raw in original[44..44 + frames * 4].chunks_exact(4) {
        let v = f32::from_le_bytes(raw.try_into().unwrap());
        for _ in 0..channels {
            match (float, bits) {
                (true, 32) => pcm.extend(v.to_le_bytes()),
                (false, 16) => pcm.extend(((v as f64 * 32768.0) as i16).to_le_bytes()),
                (false, 24) => pcm.extend(&((v as f64 * 8388608.0) as i32).to_le_bytes()[..3]),
                (false, 32) => pcm.extend(((v as f64 * 2147483648.0) as i32).to_le_bytes()),
                _ => panic!("fixture format"),
            }
        }
    }
    let tag = if float { 3u16 } else { 1u16 };
    let align = channels * (bits / 8);
    let mut fmt = Vec::new();
    fmt.extend((if extensible { 0xfffe } else { tag }).to_le_bytes());
    fmt.extend(channels.to_le_bytes());
    fmt.extend(8000u32.to_le_bytes());
    fmt.extend((8000 * u32::from(align)).to_le_bytes());
    fmt.extend(align.to_le_bytes());
    fmt.extend(bits.to_le_bytes());
    if extensible {
        fmt.extend(22u16.to_le_bytes());
        fmt.extend(bits.to_le_bytes());
        fmt.extend((if channels == 1 { 4u32 } else { 3u32 }).to_le_bytes());
        fmt.extend(tag.to_le_bytes());
        fmt.extend([0, 0, 0, 0, 16, 0, 128, 0, 0, 170, 0, 56, 155, 113]);
    }
    fn chunk(out: &mut Vec<u8>, id: &[u8; 4], data: &[u8]) {
        out.extend(id);
        out.extend((data.len() as u32).to_le_bytes());
        out.extend(data);
        if !data.len().is_multiple_of(2) {
            out.push(0);
        }
    }
    let mut out = b"RIFF\0\0\0\0WAVE".to_vec();
    chunk(&mut out, b"JUNK", &[1, 2, 3]);
    if extensible {
        chunk(&mut out, b"data", &pcm);
        chunk(&mut out, b"fmt ", &fmt);
    } else {
        chunk(&mut out, b"fmt ", &fmt);
        chunk(&mut out, b"data", &pcm);
    }
    chunk(&mut out, b"LIST", &[7; 9]);
    let size = (out.len() - 8) as u32;
    out[4..8].copy_from_slice(&size.to_le_bytes());
    out.extend(b"trailing identity bytes");
    out
}

pub mod service;
