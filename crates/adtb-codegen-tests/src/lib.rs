//! The code generated from the fixture shop's log and bundle, and from the campaign app's.

pub mod shop {
    include!(concat!(env!("OUT_DIR"), "/shop.rs"));
}

pub mod campaign {
    include!(concat!(env!("OUT_DIR"), "/campaign.rs"));
}
