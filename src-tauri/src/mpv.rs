// Android: libmpv runs inside the Kotlin plugin, driven through this bridge.
#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "android")]
pub use android::Mpv;

#[cfg(all(not(target_os = "android"), feature = "real-mpv"))]
mod real;
#[cfg(all(not(target_os = "android"), feature = "real-mpv"))]
pub use real::Mpv;

#[cfg(all(not(target_os = "android"), not(feature = "real-mpv")))]
mod stub;
#[cfg(all(not(target_os = "android"), not(feature = "real-mpv")))]
pub use stub::Mpv;
