use std::{
    fs::File,
    io::{self, Read, Seek, Write},
    path::{Path, PathBuf},
};

use anyhow::{bail, Result};
use blake3::Hash;

use crate::{age, ctx::Context, git::Error as GitError, git::Repository};

pub(crate) struct CommandContext<C: Context> {
    pub ctx: C,
}

impl<C: Context> CommandContext<C> {
    pub(crate) fn clean(&self, file: impl AsRef<Path>) -> Result<()> {
        log::info!("Encrypting file");
        let file = self.ctx.repo().workdir().join(file);

        log::debug!("Looking for saved has information. target={:?}", file,);
        let mut existing_hash = [0u8; 32];
        if let Some(hash_buffer) = self.ctx.load_sidecar(&file, "hash")? {
            existing_hash = hash_buffer.as_slice().try_into()?
        } else {
            log::debug!("No saved hash file found");
        }

        let mut hasher = blake3::Hasher::new();
        let mut contents = vec![];
        io::stdin().read_to_end(&mut contents)?;
        let hash = hasher.update(&contents).finalize();

        let old_hash = Hash::from(existing_hash);
        log::debug!(
            "Comparing hashes for file; old_hash={}, new_hash={:?}",
            old_hash.to_hex().as_str(),
            hash.to_hex().as_str()
        );

        let saved = if hash == old_hash {
            self.ctx.load_sidecar(&file, "age")?
        } else {
            None
        };

        let result = self.get_content(contents, hash, file, saved)?;
        Ok(io::stdout().write_all(&result)?)
    }

    fn get_content(
        &self,
        contents: Vec<u8>,
        hash: Hash,
        file: PathBuf,
        saved_content: Option<Vec<u8>>,
    ) -> Result<Vec<u8>> {
        if let Some(saved_content) = saved_content {
            log::debug!("File didn't change since last encryption, loading from git HEAD");
            return Ok(saved_content);
        }

        log::debug!("Encrypted content changed, checking decrypted version");
        let repo_contents = match self.ctx.repo().get_file_contents(&file) {
            Ok(v) => Some(v),
            Err(GitError::NotExist(s)) => {
                log::debug!("{}", s);
                None
            }
            Err(e) => return Err(e.into()),
        };

        if let Some(repo_contents) = repo_contents {
            let identities = self.get_identities()?;
            let mut cur = io::Cursor::new(repo_contents);
            let decrypted = age::decrypt(&identities, &mut cur)?.unwrap_or_default();
            if decrypted == contents {
                log::debug!("Decrypted content matches, using from working copy");
                self.ctx.store_sidecar(&file, "hash", hash.as_bytes())?;
                self.ctx.store_sidecar(&file, "age", cur.get_ref())?;
                return Ok(cur.into_inner());
            }
        }

        log::debug!("File changed since last encryption, re-encrypting");

        let cfg = self.ctx.config()?;
        let public_keys = cfg.get_public_keys(&file)?;

        let res = age::encrypt(public_keys, &mut &contents[..], cfg.armor)?;
        self.ctx.store_sidecar(&file, "hash", hash.as_bytes())?;
        self.ctx.store_sidecar(&file, "age", &res)?;
        Ok(res)
    }

    fn get_identities(&self) -> Result<Vec<String>> {
        log::debug!("Loading identities from config");
        let all_identities = self.ctx.repo().list_config("identity")?;
        log::debug!(
            "Loaded identities from config; identities='{:?}'",
            all_identities
        );
        Ok(all_identities)
    }

    pub(crate) fn smudge(&self, file: impl AsRef<Path>) -> Result<()> {
        log::info!("Decrypting file");
        let file = self.ctx.repo().workdir().join(file);

        let mut encrypted = vec![];
        io::stdin().read_to_end(&mut encrypted)?;
        let mut cur = io::Cursor::new(encrypted);
        let all_identities = self.get_identities()?;
        if let Some(rv) = age::decrypt(&all_identities, &mut cur)? {
            log::info!("Decrypted file");
            let mut hasher = blake3::Hasher::new();
            let hash = hasher.update(&rv).finalize();

            log::debug!("Storing hash for file; hash={:?}", hash.to_hex().as_str(),);
            self.ctx.store_sidecar(&file, "hash", hash.as_bytes())?;
            self.ctx.store_sidecar(&file, "age", cur.get_ref())?;

            Ok(io::stdout().write_all(&rv)?)
        } else {
            bail!("Input isn't encrypted")
        }
    }

    pub(crate) fn textconv(&self, path: impl AsRef<Path>) -> Result<()> {
        log::info!("Decrypting file to show in diff");

        let all_identities: Vec<String> = self
            .ctx
            .age_identities()
            .list()?
            .into_iter()
            .map(|i| i.path)
            .collect();

        let mut f = File::open(path)?;
        let result = if let Some(rv) = age::decrypt(&all_identities, &mut f)? {
            log::info!("Decrypted file to show in diff");
            rv
        } else {
            log::info!("File isn't encrypted, probably a working copy; showing as is.");
            f.rewind()?;
            let mut buff = vec![];
            f.read_to_end(&mut buff)?;
            buff
        };
        Ok(io::stdout().write_all(&result)?)
    }
}
