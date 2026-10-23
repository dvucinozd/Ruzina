//! Controller-profile upload into `/controllers/<id>/profile.s3bin`
//! (RP-02, M2.5 `controller_profile_storage_install`).
//!
//! The uploaded bytes are accepted only if `Profile::parse` accepts them, the
//! same parser the registry scan and activation use, so nothing a scan would
//! reject can be installed. The install is a file swap that a power cut at
//! any step leaves as either the old profile or the new one:
//!
//! 1. recover any interrupted install of this id;
//! 2. write `profile.upload`, flush it, and read it back byte for byte;
//! 3. move the current profile to `profile.backup`, rename the upload into
//!    place and read the result back again;
//! 4. drop the backup.
//!
//! Any failure after step 2 restores the backup. The caller rescans the
//! registry afterwards so a connected controller can pick the profile up.

use crate::{
    CONTROLLER_PROFILE_FILENAME, CONTROLLER_PROFILE_ROOT, ProfilePath, ProfileScanBuffers,
};
use ruzina_controller_profile::{MAX_PROFILE_SIZE, ParseError, Profile, ProfileId};
use ruzina_media_fs::{AsyncWritableFileSystem, FileHandle};

pub const CONTROLLER_PROFILE_UPLOAD_FILENAME: &str = "profile.upload";
pub const CONTROLLER_PROFILE_BACKUP_FILENAME: &str = "profile.backup";
/// M2.5 Web upload bounds: 32..=16384 bytes.
pub const PROFILE_UPLOAD_MIN: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstallStep {
    Directory,
    Recover,
    WriteUpload,
    VerifyUpload,
    Backup,
    Promote,
    VerifyTarget,
    Sync,
}

#[derive(Debug, Eq, PartialEq)]
pub enum InstallError<E> {
    InvalidId,
    Size,
    InvalidProfile(ParseError),
    /// A profile with this id exists and overwrite was not requested.
    Exists,
    /// `error` is `None` when the data read back did not match.
    Io {
        step: InstallStep,
        error: Option<E>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InstalledProfile {
    pub id: ProfileId,
    pub size: usize,
    pub version: u16,
    pub crc: u32,
    pub vid: u16,
    pub pid: u16,
    /// True when an existing profile was replaced.
    pub replaced: bool,
}

struct Paths {
    directory: ProfilePath,
    target: ProfilePath,
    upload: ProfilePath,
    backup: ProfilePath,
}

impl Paths {
    fn new(id: &ProfileId) -> Option<Self> {
        Some(Self {
            directory: ProfilePath::directory(id.as_str())?,
            target: ProfilePath::file(id.as_str(), CONTROLLER_PROFILE_FILENAME)?,
            upload: ProfilePath::file(id.as_str(), CONTROLLER_PROFILE_UPLOAD_FILENAME)?,
            backup: ProfilePath::file(id.as_str(), CONTROLLER_PROFILE_BACKUP_FILENAME)?,
        })
    }
}

fn io<E>(step: InstallStep) -> impl FnOnce(E) -> InstallError<E> {
    move |error| InstallError::Io {
        step,
        error: Some(error),
    }
}

impl ProfileScanBuffers {
    /// Installs `data` as profile `id` (see the module docs).
    pub async fn install<F>(
        &mut self,
        filesystem: &mut F,
        id: &str,
        data: &[u8],
        overwrite: bool,
    ) -> Result<InstalledProfile, InstallError<F::Error>>
    where
        F: AsyncWritableFileSystem,
    {
        let id = ProfileId::parse(id).map_err(|_| InstallError::InvalidId)?;
        if !(PROFILE_UPLOAD_MIN..=MAX_PROFILE_SIZE).contains(&data.len()) {
            return Err(InstallError::Size);
        }
        let profile = Profile::parse(data).map_err(InstallError::InvalidProfile)?;
        let paths = Paths::new(&id).ok_or(InstallError::InvalidId)?;
        filesystem
            .create_directory(CONTROLLER_PROFILE_ROOT)
            .await
            .map_err(io(InstallStep::Directory))?;
        filesystem
            .create_directory(paths.directory.as_str())
            .await
            .map_err(io(InstallStep::Directory))?;
        self.recover_paths(filesystem, &paths)
            .await
            .map_err(io(InstallStep::Recover))?;

        let had_target = exists(filesystem, &paths.target).await;
        if had_target && !overwrite {
            return Err(InstallError::Exists);
        }

        if let Err(error) = write_file(filesystem, paths.upload.as_str(), data).await {
            let _ = filesystem.remove(paths.upload.as_str()).await;
            return Err(InstallError::Io {
                step: InstallStep::WriteUpload,
                error,
            });
        }
        if !self.matches(filesystem, &paths.upload, data).await {
            let _ = filesystem.remove(paths.upload.as_str()).await;
            return Err(InstallError::Io {
                step: InstallStep::VerifyUpload,
                error: None,
            });
        }

        if had_target {
            let _ = filesystem.remove(paths.backup.as_str()).await;
            if let Err(error) = filesystem
                .rename(paths.target.as_str(), paths.backup.as_str())
                .await
            {
                let _ = filesystem.remove(paths.upload.as_str()).await;
                return Err(io(InstallStep::Backup)(error));
            }
        }
        if let Err(error) = filesystem
            .rename(paths.upload.as_str(), paths.target.as_str())
            .await
        {
            if had_target {
                let _ = filesystem
                    .rename(paths.backup.as_str(), paths.target.as_str())
                    .await;
            }
            let _ = filesystem.remove(paths.upload.as_str()).await;
            return Err(io(InstallStep::Promote)(error));
        }
        if !self.matches(filesystem, &paths.target, data).await {
            let _ = filesystem.remove(paths.target.as_str()).await;
            if had_target {
                let _ = filesystem
                    .rename(paths.backup.as_str(), paths.target.as_str())
                    .await;
            }
            return Err(InstallError::Io {
                step: InstallStep::VerifyTarget,
                error: None,
            });
        }
        let _ = filesystem.remove(paths.backup.as_str()).await;
        filesystem.sync().await.map_err(io(InstallStep::Sync))?;
        Ok(InstalledProfile {
            id,
            size: data.len(),
            version: profile.version(),
            crc: profile.crc(),
            vid: profile.vid(),
            pid: profile.pid(),
            replaced: had_target,
        })
    }

    /// Completes or rolls back an interrupted install of `id`: a valid
    /// profile wins, otherwise a valid backup is restored; leftovers go.
    pub async fn recover_profile<F>(
        &mut self,
        filesystem: &mut F,
        id: &str,
    ) -> Result<(), InstallError<F::Error>>
    where
        F: AsyncWritableFileSystem,
    {
        let id = ProfileId::parse(id).map_err(|_| InstallError::InvalidId)?;
        let paths = Paths::new(&id).ok_or(InstallError::InvalidId)?;
        self.recover_paths(filesystem, &paths)
            .await
            .map_err(io(InstallStep::Recover))
    }

    async fn recover_paths<F>(&mut self, filesystem: &mut F, paths: &Paths) -> Result<(), F::Error>
    where
        F: AsyncWritableFileSystem,
    {
        if exists(filesystem, &paths.target).await {
            if !self.valid(filesystem, &paths.target).await
                && self.valid(filesystem, &paths.backup).await
            {
                filesystem.remove(paths.target.as_str()).await?;
                filesystem
                    .rename(paths.backup.as_str(), paths.target.as_str())
                    .await?;
            }
            let _ = filesystem.remove(paths.upload.as_str()).await;
            let _ = filesystem.remove(paths.backup.as_str()).await;
            return Ok(());
        }
        if exists(filesystem, &paths.backup).await {
            filesystem
                .rename(paths.backup.as_str(), paths.target.as_str())
                .await?;
        }
        let _ = filesystem.remove(paths.upload.as_str()).await;
        Ok(())
    }

    /// Reads `path` into the scratch; `None` when unreadable or too large.
    async fn read<F>(&mut self, filesystem: &mut F, path: &ProfilePath) -> Option<usize>
    where
        F: AsyncWritableFileSystem,
    {
        let mut file = filesystem.open_read(path.as_str()).await.ok()?;
        let len = usize::try_from(file.len())
            .ok()
            .filter(|len| *len <= MAX_PROFILE_SIZE);
        let mut offset = 0;
        let complete = match len {
            Some(len) => loop {
                if offset == len {
                    break true;
                }
                match filesystem
                    .read(&mut file, &mut self.profile_storage[offset..len])
                    .await
                {
                    Ok(read) if read != 0 && read <= len - offset => offset += read,
                    _ => break false,
                }
            },
            None => false,
        };
        let closed = filesystem.close_file(file).await.is_ok();
        (complete && closed).then_some(offset)
    }

    async fn valid<F>(&mut self, filesystem: &mut F, path: &ProfilePath) -> bool
    where
        F: AsyncWritableFileSystem,
    {
        self.read(filesystem, path)
            .await
            .is_some_and(|len| Profile::parse(&self.profile_storage[..len]).is_ok())
    }

    async fn matches<F>(&mut self, filesystem: &mut F, path: &ProfilePath, data: &[u8]) -> bool
    where
        F: AsyncWritableFileSystem,
    {
        self.read(filesystem, path)
            .await
            .is_some_and(|len| &self.profile_storage[..len] == data)
    }
}

async fn exists<F: AsyncWritableFileSystem>(filesystem: &mut F, path: &ProfilePath) -> bool {
    filesystem.stat(path.as_str()).await.is_ok()
}

/// `Err(None)` for a short write or a backend that lied about its count.
async fn write_file<F: AsyncWritableFileSystem>(
    filesystem: &mut F,
    path: &str,
    data: &[u8],
) -> Result<(), Option<F::Error>> {
    let mut file = filesystem.open_write(path, true).await.map_err(Some)?;
    let written = match filesystem.write_all(&mut file, data).await {
        Ok(()) => filesystem.flush_file(&mut file).await.map_err(Some),
        Err(ruzina_media_fs::IoContractError::Inner(error)) => Err(Some(error)),
        Err(_) => Err(None),
    };
    let closed = filesystem.close_file(file).await.map_err(Some);
    written.and(closed)
}
