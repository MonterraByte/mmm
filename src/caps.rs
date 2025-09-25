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

use std::marker::PhantomData;

use rustix::thread;
use rustix::thread::{CapabilitySet, CapabilitySets};

const CAPS_DISABLED: CapabilitySets = CapabilitySets {
    effective: CapabilitySet::empty(),
    permitted: CapabilitySet::SYS_ADMIN,
    inheritable: CapabilitySet::empty(),
};

const CAPS_ENABLED: CapabilitySets = CapabilitySets {
    effective: CapabilitySet::SYS_ADMIN,
    permitted: CapabilitySet::SYS_ADMIN,
    inheritable: CapabilitySet::empty(),
};

pub fn init() {
    thread::clear_ambient_capability_set().expect("clear ambient capabilities");

    let current_caps = thread::capabilities(None).expect("get current capabilities");
    if !current_caps.permitted.contains(CapabilitySet::SYS_ADMIN) {
        eprintln!(
            "The SYS_ADMIN capability, required for mounting and unmounting filesystems, is missing.\nRun `setcap cap_sys_admin=p '{}'` as root to grant it to this program, then try again.",
            std::env::current_exe()
                .expect("get executable path")
                .canonicalize()
                .expect("canonicalize executable path")
                .display()
        );
        std::process::exit(1);
    }

    lower();
}

fn lower() {
    thread::set_capabilities(None, CAPS_DISABLED).expect("drop capabilities");
}

pub struct ElevatedCaps {
    // Each thread has its own capability set, so this struct must not be `Send`,
    // to correctly drop the capabilities that it raised.
    //
    // Until the negative_impls feature is stabilized (https://github.com/rust-lang/rust/issues/68318),
    // using `PhantomData` is the nicer way to guarantee this.
    _marker: PhantomData<*const ()>,
}

impl ElevatedCaps {
    pub fn raise() -> Self {
        thread::set_capabilities(None, CAPS_ENABLED).expect("raise capabilities");
        Self { _marker: PhantomData }
    }
}

impl Drop for ElevatedCaps {
    fn drop(&mut self) {
        lower();
    }
}
