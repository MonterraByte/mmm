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

mod caps;
mod instance;
mod mount;
mod namespace;
mod staging;

use std::io::Read;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;

use clap::Parser;
use signal_hook::consts::SIGINT;

use mmm_core::file_tree::{self, FileTreeDisplayKind};

use crate::instance::DeployInstance;
use crate::mount::{MountMethod, MountMethodChoice, OverlayMount};
use crate::staging::build_staging_tree;

#[derive(Parser)]
struct Args {
    #[arg(value_enum, short, long, required = false, default_value_t)]
    mount_method: MountMethodChoice,
    instance_path: PathBuf,
    game_path: PathBuf,
    #[arg(short = 'x', long)]
    exec: Option<PathBuf>,
    #[arg(short, long)]
    profile: Option<String>,
}

fn main() {
    caps::init();
    let args = Args::parse();
    let mount_method = args.mount_method.to_mount_method();
    if matches!(mount_method, MountMethod::UserNamespace) && args.exec.is_none() {
        eprintln!("--exec is required when using user namespaces");
        std::process::exit(1);
    }

    let mods = DeployInstance::open(&args.instance_path, args.profile.as_deref()).expect("failed to open instance");
    let tree = file_tree::build_path_tree(&mods).unwrap();
    ptree::print_tree(&file_tree::FileTreeDisplay::new(
        &tree,
        &mods,
        FileTreeDisplayKind::Conflicts,
    ))
    .unwrap();

    if matches!(mount_method, MountMethod::UserNamespace) {
        namespace::enter_namespace().expect("failed to enter user namespace");
    }

    let staging_dir = build_staging_tree(&tree, &mods).expect("build staging tree");
    println!("Built staging tree at '{}'", staging_dir.path().display());

    let game_path = args.game_path.canonicalize().expect("canonicalize game path");
    let overlay_mount = OverlayMount::new(staging_dir.path(), &game_path).expect("mount overlay");
    println!("Mounted overlay over {}", overlay_mount.path().display());

    if let Some(mut exe) = args.exec {
        if exe.is_relative() {
            exe = args.game_path.join(exe);
        }
        run_game_and_wait(&exe);
    } else {
        println!("\nPress Control + C to unmount the overlay");
        wait_for_sigterm();
    }

    overlay_mount.unmount().expect("unmounting failed");
    staging_dir.unmount().expect("unmounting failed");
    println!("\nUnmount successful");
}

fn run_game_and_wait(exe: &Path) {
    let mut game = Command::new(exe)
        .current_dir(exe.parent().expect("executable has parent directory"))
        .spawn()
        .expect("failed to execute game");

    let exe_name = exe.file_name().expect("executable has file name").display();
    println!("\nWaiting for {} to exit", exe_name);

    let exit_status = game.wait().expect("waitpid failed");
    match exit_status.code() {
        Some(code) => {
            if code != 0 {
                eprintln!("{} exited with code {}", exe_name, code);
            }
        }
        None => eprintln!("{} was terminated by a signal", exe_name),
    }
}

fn wait_for_sigterm() {
    let (mut read, write) = UnixStream::pair().expect("create socket pair");
    let handler = signal_hook::low_level::pipe::register(SIGINT, write).expect("register SIGTERM handler");

    let mut buff = [0];
    read.read_exact(&mut buff).expect("read from the self-pipe");

    signal_hook::low_level::unregister(handler);
}
