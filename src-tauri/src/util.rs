use std::{
	fs::DirEntry,
	sync::{atomic::AtomicBool, Arc, PoisonError},
};

use parking_lot::{
	RwLock, RwLockReadGuard, RwLockWriteGuard
};

use tauri::Webview;

pub mod path {
	use serde::{de::Visitor, Deserialize, Serialize};
	use std::{fmt::Debug, path::PathBuf};

	pub fn canonicalize(path: PathBuf) -> PathBuf {
		dunce::canonicalize(path.clone()).unwrap_or(path)
	}

	#[cfg(not(target_os = "windows"))]
	pub fn normalize(path: PathBuf) -> PathBuf {
		canonicalize(path)
	}

	#[cfg(target_os = "windows")]
	pub fn normalize(path: PathBuf) -> PathBuf {
		match dunce::canonicalize(&path) {
			Ok(canonicalized) => PathBuf::from(
				canonicalized
					.to_string_lossy()
					.to_string()
					.replace('\\', "/"),
			),
			Err(_) => path,
		}
	}

	#[derive(Clone)]
	pub struct NormalizedPathBuf {
		pub normalized: PathBuf,
		path: PathBuf,
	}
	impl std::ops::Deref for NormalizedPathBuf {
		type Target = PathBuf;
		fn deref(&self) -> &Self::Target {
			&self.path
		}
	}
	impl From<PathBuf> for NormalizedPathBuf {
		fn from(path: PathBuf) -> Self {
			Self {
				path: path.clone(),
				normalized: normalize(path),
			}
		}
	}
	impl From<&PathBuf> for NormalizedPathBuf {
		fn from(path: &PathBuf) -> Self {
			let path = path.to_owned();
			Self {
				path: path.clone(),
				normalized: normalize(path),
			}
		}
	}
	impl From<String> for NormalizedPathBuf {
		fn from(path: String) -> Self {
			let path = PathBuf::from(path);
			Self {
				path: path.clone(),
				normalized: normalize(path),
			}
		}
	}
	impl From<&str> for NormalizedPathBuf {
		fn from(path: &str) -> Self {
			let path = PathBuf::from(path);
			Self {
				path: path.clone(),
				normalized: normalize(path),
			}
		}
	}
	impl Debug for NormalizedPathBuf {
		fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
			self.path.fmt(f)
		}
	}

	impl Serialize for NormalizedPathBuf {
		fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
		where
			S: serde::Serializer,
		{
			serializer.serialize_str(&self.normalized.to_string_lossy())
		}
	}

	struct NormalizedPathBufVisitor;
	impl<'de> Visitor<'de> for NormalizedPathBufVisitor {
		type Value = String;

		fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
			formatter.write_str("a string")
		}
	}
	impl<'de> Deserialize<'de> for NormalizedPathBuf {
		fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
		where
			D: serde::Deserializer<'de>,
		{
			Ok(NormalizedPathBuf::from(
				deserializer.deserialize_string(NormalizedPathBufVisitor)?,
			))
		}
	}
}

pub(crate) fn prompt_path_dialog(
	callback: String,
	error: String,
	webview: &mut Webview,
	multiple: bool,
	directory: bool,
	save: bool,
	default_path: Option<String>,
) -> Result<(), String> {
	use rfd::FileDialog;
	use std::path::PathBuf;

	tauri::execute_promise(webview, move || {

		let builder = FileDialog::new();
		
	 	let builder = if let Some(default_path) = default_path {
			let mut path = PathBuf::from(default_path);
			if path.is_file() { path.pop(); }
			builder.set_directory(&path)
		} else {
			builder
		};

		if save {
			Ok(builder.save_file().map(|x| vec![x]))
		} else if directory {
			Ok(builder.pick_folder().map(|x| vec![x]))
		} else if multiple {
			Ok(builder.pick_files())
		} else {
			Ok(builder.pick_file().map(|x| vec![x]))
		}

	}, callback, error);

	Ok(())
}

pub(crate) fn get_modified_time(entry: &DirEntry) -> Result<u64, anyhow::Error> {
	Ok(entry.metadata()?.modified()?.elapsed()?.as_secs())
}

// pepega
#[cfg(not(debug_assertions))]
pub(crate) type RwLockDebug<T> = RwLock<T>;

#[cfg(debug_assertions)]
#[derive(Default)]
pub(crate) struct RwLockDebug<T> {
	inner: RwLock<T>,
	backtrace: Arc<RwLock<Option<(backtrace::Backtrace, std::time::Instant)>>>,
}
#[cfg(debug_assertions)]
impl<T> RwLockDebug<T> {
	pub(crate) fn new(val: T) -> Self {
		Self {
			inner: RwLock::new(val),
			backtrace: Arc::new(RwLock::new(None)),
		}
	}

	fn backtrace(&self) {
		*self.backtrace.write() =
			Some((backtrace::Backtrace::new(), std::time::Instant::now()));
	}

	fn watchdog(&self, calling_backtrace: backtrace::Backtrace) -> Arc<AtomicBool> {
		let success = Arc::new(AtomicBool::new(false));
		{
			let started = std::time::Instant::now();
			let backtrace = self.backtrace.clone();
			let success = success.clone();
			std::thread::spawn(move || loop {
				if success.load(std::sync::atomic::Ordering::Acquire) {
					break;
				} else if started.elapsed().as_secs() >= 3 {
					println!("[RwLock] POTENTIAL DEADLOCK!");
					println!("[RwLock] Invoked by:");
					println!("{:#?}", calling_backtrace);

					let (backtrace, mut timestamp) = match match backtrace.try_write() {
						Some(mut backtrace_w) => backtrace_w.take(),
						None => (&*backtrace.read()).clone(),
					} {
						Some(backtrace) => backtrace,
						None => return println!("[RwLock] Locked by: UNKNOWN"),
					};

					println!("[RwLock] Locked {} before by:", {
						timestamp = timestamp + std::time::Duration::from_secs(3);
						let elapsed = timestamp.elapsed();
						if elapsed.as_secs() != 0 {
							elapsed.as_secs_f64().to_string() + "s"
						} else if elapsed.as_millis() != 0 {
							elapsed.as_millis().to_string() + "ms"
						} else if elapsed.as_micros() != 0 {
							elapsed.as_micros().to_string() + "us"
						} else {
							elapsed.as_nanos().to_string() + "ns"
						}
					});
					println!("{:#?}", backtrace);

					break;
				}
			});
		}
		success
	}

	pub(crate) fn read(
		&self,
	) -> RwLockReadGuard<'_, T> {
		let success = self.watchdog(backtrace::Backtrace::new());
		let lock = self.inner.read();
		success.store(true, std::sync::atomic::Ordering::Release);
		self.backtrace();
		lock
	}

	pub(crate) fn write(
		&self,
	) -> RwLockWriteGuard<'_, T> {
		let success = self.watchdog(backtrace::Backtrace::new());
		let lock = self.inner.write();
		success.store(true, std::sync::atomic::Ordering::Release);
		self.backtrace();
		lock
	}
}
#[cfg(debug_assertions)]
impl<T> std::ops::Deref for RwLockDebug<T> {
	type Target = RwLock<T>;
	fn deref(&self) -> &Self::Target {
		&self.inner
	}
}
#[cfg(debug_assertions)]
impl<T> std::ops::DerefMut for RwLockDebug<T> {
	fn deref_mut(&mut self) -> &mut Self::Target {
		&mut self.inner
	}
}

pub(crate) struct ThreadWatchdog {
	callback: Box<dyn Fn() + Sync + Send + 'static>,
}
impl ThreadWatchdog {
	pub(crate) fn new<F>(f: F) -> Self
	where
		F: Fn() + Sync + Send + 'static,
	{
		Self {
			callback: Box::new(f),
		}
	}
}
impl Drop for ThreadWatchdog {
	fn drop(&mut self) {
		(self.callback)();
	}
}
