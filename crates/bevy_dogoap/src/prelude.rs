//! Everything you need to to use `bevy_dogoap`

// TODO change to upstream once available
pub use bevy_trait_query::RegisterExt;

pub use dogoap::prelude::{Action, Compare, Datum, Goal, LocalState, Mutator};

pub use crate::{
    create_planner,
    planner::IsPlanning,
    planner::{Planner, UpdatePlan},
    register_actions, register_components,
};

pub use crate::plugin::DogoapPlugin;

pub use crate::traits::{
    ActionComponent, DatumComponent, EnumDatum, InserterComponent, MutatorTrait, Precondition,
};

pub use dogoap_macros::{ActionComponent, DatumComponent, EnumComponent, EnumDatum};

pub(crate) use bevy_app::prelude::*;
pub(crate) use bevy_ecs::prelude::*;
pub(crate) use bevy_log::prelude::*;
pub(crate) use bevy_reflect::prelude::*;
