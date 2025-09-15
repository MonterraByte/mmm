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

mod mods;
mod mount;
mod staging;

use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;

use crate::mods::Mods;
use crate::staging::build_staging_tree;

#[derive(Parser)]
struct Args {
    path: PathBuf,
}

fn main() {
    let args = Args::parse();

    let base_dir = args.path;
    let mods = Mods::read(&base_dir).expect("failed reading mods");

    let tree = mods::build_path_tree(&mods).unwrap();
    ptree::print_tree(&mods::FileTreeDisplay::new(&tree, &mods)).unwrap();

    let staging_dir = build_staging_tree(&tree, &mods).expect("build staging tree");
    println!("Built staging tree at '{}'", staging_dir.path().display());

    std::thread::sleep(Duration::from_secs(30));

    staging_dir.unmount().expect("unmounting failed");
}
