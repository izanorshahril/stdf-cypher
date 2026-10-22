//! stdf-cypher: open STDF v4 analytics.
#![allow(
    clippy::neg_cmp_op_on_partial_ord,
    clippy::type_complexity,
    clippy::too_many_arguments,
    clippy::needless_range_loop,
    clippy::unnecessary_sort_by,
    clippy::unnecessary_unwrap
)]
pub mod analysis;
pub mod api;
pub mod data;
pub mod datalog;
pub mod desktop;
pub mod merge;
pub mod ml;
pub mod report;
pub mod sample;
pub mod server;
pub mod split;
pub mod stdf;
pub mod viz;
