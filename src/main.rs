#![warn(clippy::cargo)]
#![allow(clippy::cargo_common_metadata)]
#![warn(clippy::pedantic)]
#![allow(clippy::similar_names)]

use std::fs::{canonicalize, remove_file, OpenOptions};
use std::io::{copy, Seek, SeekFrom};
use std::os::fd::AsRawFd;
use std::os::unix::prelude::OpenOptionsExt;
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use clap::Parser;
use nix::fcntl::{fallocate, FallocateFlags};
use nix::libc::{O_NOFOLLOW, S_IFMT, S_IFREG};
use nix::sys::stat::fstat;
use nix::sys::statfs::{fstatfs, BTRFS_SUPER_MAGIC};
use nix::unistd::fsync;

use crate::default_read::DefaultRead;

mod default_read;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Args {
    /// Path to the file to defragment
    path: PathBuf,
    // TODO: select how to defragment: dedupe or reflink
    // TODO: option to check whether defragmenting is beneficial
}

fn main() -> Result<()> {
    let args = Args::parse();

    defrag(&args.path)
}

fn defrag(path: &PathBuf) -> Result<()> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(O_NOFOLLOW)
        .open(path)?;

    let fd = file.as_raw_fd();
    let statfs = fstatfs(&fd)?;
    if statfs.filesystem_type() != BTRFS_SUPER_MAGIC {
        bail!("filesystem is not btrfs");
    }

    let file_stat = fstat(fd)?;
    if file_stat.st_mode & S_IFMT != S_IFREG {
        bail!("'{}' is not a regular file", path.display());
    }

    // Create the workfile in the same directory as the file to be defragmented.
    let realpath = canonicalize(path)?;
    let dirpath = realpath.as_path().parent().unwrap();
    let workfile_path = Path::join(dirpath, format!(".defrag.{}", file_stat.st_ino));
    let mut workfile = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&workfile_path)?;

    // We don't actually need the workfile linked at any point, we only need the fd.
    // So unlink it right away.
    remove_file(&workfile_path)?;

    let workfile_fd = workfile.as_raw_fd();

    // Preallocating is the magic by which max-sized file extents are created.
    // Currently, on kernel 6.1.12, this results in 256 MiB sized file extents.
    println!("Preallocating {} bytes...", file_stat.st_size);
    fallocate(workfile_fd, FallocateFlags::empty(), 0, file_stat.st_size)?;

    // Wrap the file in a struct, so that `std::io::copy` doesn't specialize to
    // using `copy_file_range`. `copy_file_range` copies the existing extents using
    // reflinks, which does not accomplish our goal of defragmenting the file.
    println!("Copying to work file...");
    copy(&mut DefaultRead(&file), &mut workfile)?;

    // We need fsync in order to cause the preallocated file extents to be replaced
    // by ones with data. Without this, the defragmented file won't receive the
    // extents we created in the work file.
    println!("Fsync...");
    fsync(workfile_fd)?;

    file.seek(SeekFrom::Start(0))?;
    workfile.seek(SeekFrom::Start(0))?;

    // Here we do require the `copy_file_range` specialization in order to copy
    // using reflinks, so that the defragmented file will receive the extents we
    // created in the work file.
    println!("Copying back using reflinks...");
    copy(&mut workfile, &mut file)?;

    println!("Done!");

    Ok(())
}
