//! Local replacement for librqbit's SHA-1 wrapper (same API, same version).
//!
//! Upstream hashes with OpenSSL (`crypto-hash`) or aws-lc, and neither
//! cross-compiles for Android without extra native toolchains. The pure-Rust
//! `sha1` crate is slower, but a phone only ever verifies one stream's worth
//! of pieces at a time.

use sha1::{Digest, Sha1 as RustSha1};

pub trait ISha1 {
    fn new() -> Self;
    fn update(&mut self, buf: &[u8]);
    fn finish(self) -> [u8; 20];
}

pub struct Sha1 {
    inner: RustSha1,
}

impl ISha1 for Sha1 {
    fn new() -> Self {
        Self {
            inner: RustSha1::new(),
        }
    }

    fn update(&mut self, buf: &[u8]) {
        self.inner.update(buf);
    }

    fn finish(self) -> [u8; 20] {
        self.inner.finalize().into()
    }
}
