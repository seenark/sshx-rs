mod connection;
mod connection_view;
mod doctor;
mod host_create;
mod host_edit;
mod hosts;
mod matcher;
mod pair_setup;
mod pairs;
mod review;
mod setup;
mod terminal;
mod tunnels;
mod widgets;

pub(crate) use connection::{ConnectionMode, connection_workspace};
pub(crate) use doctor::{DoctorAction, doctor_workspace};
pub(crate) use host_create::{HostCreateAction, host_create_workspace};
pub(crate) use host_edit::{HostEditAction, HostEditState, host_edit_workspace};
pub(crate) use hosts::{HostsState, Selection, browse_hosts, select};
pub(crate) use pair_setup::{PairSetupAction, PairSetupDraft, pair_setup_workspace};
pub(crate) use pairs::{PairWorkspaceAction, pairs_workspace};
pub(crate) use review::{MutationReviewAction, MutationReviewMode, mutation_review_workspace};
pub(crate) use setup::setup_workspace;
pub(crate) use tunnels::tunnels_workspace;

pub(crate) const CANCELLED: &str = "PICKER_CANCELLED";
pub(crate) const BACK: &str = "PICKER_BACK";
