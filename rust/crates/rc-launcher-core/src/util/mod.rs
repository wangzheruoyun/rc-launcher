//! Performance & memory optimisation utilities (task 25).
//!
//! * [`bufpool`] — object pools for reusable byte buffers and arbitrary typed
//!   objects, so the hot paths (streaming download writes, `.tar.xz`
//!   extraction, AWT frame blits) stop paying per-call allocation cost and
//!   reuse capacity instead.
//!
//! This mirrors two patterns borrowed from the reference launchers / servers in
//! `~/com.rc.launcher/snapshots/`:
//! * **cuberite** keeps a bounded, reused `RegionCache` rather than allocating
//!   fresh per chunk — fixed working sets beat unbounded growth on low-end
//!   hardware.
//! * **MCTier** shuttles bytes through `Arc`-shared, channel-reused buffers
//!   between its native core and the UI instead of cloning on every crossing.

pub mod bufpool;

pub use bufpool::{BufPool, ObjectPool, PooledBuf, PooledObject};
