//! Engine REST handlers.
//!
//! - `identity`, `capabilities`: full implementations (sub-project 1).
//! - `payloads`, `forkchoice`: full implementations (sub-project 2).
//! - `bodies`, `blobs`: full implementations (sub-project 3).

pub mod blobs;
pub mod bodies;
pub mod capabilities;
pub mod forkchoice;
pub mod helpers;
pub mod identity;
pub mod payloads;
