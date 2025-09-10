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

use std::fs;
use std::path::PathBuf;

use clap::Parser;

#[derive(Parser)]
struct Args {
    path: PathBuf,
}

fn main() {
    let args = Args::parse();

    let base_dir = args.path;
    let mods_dir = base_dir.join("mods");
    let mods: Vec<String> = {
        let mut file = fs::File::open(base_dir.join("mods.json")).expect("open mods json");
        serde_json::from_reader(&mut file).expect("deserialize mods json")
    };

    let tree = mods::build_path_tree(&mods_dir, &mods).unwrap();
    ptree::print_tree(&mods::FileTreeDisplay::new(&tree, &mods)).unwrap();
}
