//! Embedded infrastructure templates shipped with the mua workspace.

/// Default stage AFB template for background/effect injection.
pub const ST_DUMMY_AFB: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/st_dummy.afb"));

/// Default notes-field AFB template copied verbatim during stage export.
pub const NF_DUMMY_AFB: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/nf_dummy.afb"));
