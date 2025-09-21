// Copyright © 2025 Joaquim Monteiro
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.

#![forbid(unsafe_code)]

mod caps;
mod mods;
mod mount;
mod staging;

use std::io::Read;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use clap::Parser;
use signal_hook::consts::SIGINT;

use crate::mods::Mods;
use crate::mount::OverlayMount;
use crate::staging::build_staging_tree;

#[derive(Parser)]
struct Args {
    instance_path: PathBuf,
    game_path: PathBuf,
}

fn main() {
    caps::init();
    let args = Args::parse();

    let mods = Mods::read(&args.instance_path).expect("failed reading mods");
    let tree = mods::build_path_tree(&mods).unwrap();
    ptree::print_tree(&mods::FileTreeDisplay::new(&tree, &mods)).unwrap();

    let staging_dir = build_staging_tree(&tree, &mods).expect("build staging tree");
    println!("Built staging tree at '{}'", staging_dir.path().display());

    let game_path = args.game_path.canonicalize().expect("canonicalize game path");
    let overlay_mount = OverlayMount::new(staging_dir.path(), &game_path).expect("mount overlay");
    println!("Mounted overlay over {}", overlay_mount.path().display());

    println!("\nPress Control + C to unmount the overlay");
    wait_for_sigterm();

    overlay_mount.unmount().expect("unmounting failed");
    staging_dir.unmount().expect("unmounting failed");
    println!("\nUnmount successful");
}

fn wait_for_sigterm() {
    let (mut read, write) = UnixStream::pair().expect("create socket pair");
    let handler = signal_hook::low_level::pipe::register(SIGINT, write).expect("register SIGTERM handler");

    let mut buff = [0];
    read.read_exact(&mut buff).expect("read from the self-pipe");

    signal_hook::low_level::unregister(handler);
}
