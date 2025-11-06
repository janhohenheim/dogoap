//! Types regarding planning

use crate::prelude::*;
use bevy_ecs::entity_disabling::Disabled;
use bevy_platform::collections::HashMap;
use core::fmt;

#[cfg(feature = "compute-pool")]
use {bevy_tasks::AsyncComputeTaskPool, crossbeam_channel::Receiver};

use dogoap::prelude::*;

// TODO can we replace this with ActionComponent perhaps? Should be able to
type ActionsMap = HashMap<String, (Action, Box<dyn InserterComponent>)>;

type DatumComponents = Vec<Box<dyn DatumComponent>>;

/// Our main struct for handling the planning within Bevy, keeping track of added
/// [`Action`]s, [`DatumComponent`]s, and some options for controlling the execution
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct Planner {
    /// Our current state used for planning, updated by [`update_planner_local_state`] which reads
    /// the current state from our Bevy world and updates it accordingly
    pub state: LocalState,
    /// A Vector of all possible [`Goal`], ordered by priority.
    pub goals: Vec<Goal>,
    /// What [`Action`] we're currrently trying to execute
    pub current_action: Option<Action>,
    /// The currently executed plan
    pub current_plan: Option<Plan>,

    // TODO figure out how to get reflect to work, if possible
    #[reflect(ignore)]
    actions_map: ActionsMap,
    /// Internal prepared vector of just [`Action`]
    actions_for_dogoap: Vec<Action>,
}

impl fmt::Debug for Planner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "State: {:#?}\nGoals: {:#?}\nActions: {:#?}\nPossible Goals:{:#?}\n",
            self.state, self.goals, self.actions_for_dogoap, self.goals
        )
    }
}

/// When we're not using `AsyncComputeTaskPool` + `Task`, we define our own so we can replace less code later
#[cfg(not(feature = "compute-pool"))]
struct Receiver<T>(T);

/// This Component holds to-be-processed data for `make_plan`
/// We do it in a asyncronous manner as `make_plan` blocks and if it takes 100ms, we'll delay frames
/// by 100ms...
#[derive(Component)]
pub(crate) struct PlanReceiver(Receiver<Option<Plan>>);

/// This Component gets added when the planner for an Entity is currently planning,
/// and removed once a plan has been created. Normally this will take under 1ms,
/// but if you have lots of actions and possible states, it can take longer
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct IsPlanning;

impl Planner {
    /// Creates a new [`Planner`] from the given components, goals, and actions map.
    pub fn new(components: DatumComponents, goals: Vec<Goal>, actions_map: ActionsMap) -> Self {
        let mut actions_for_dogoap: Vec<Action> = vec![];

        for (action, _component) in actions_map.values() {
            actions_for_dogoap.push(action.clone());
        }

        let mut state = LocalState::new();

        for component in components.iter() {
            state
                .data
                .insert(component.field_key(), component.field_value());
        }

        Self {
            state,
            goals,
            actions_map,
            current_action: None,
            current_plan: None,
            actions_for_dogoap,
        }
    }
}

/// This system "syncs" our [`DatumComponent`]s with the `LocalState` in the [`Planner`]
pub(crate) fn update_planner_local_state(
    local_field_components: Query<&dyn DatumComponent>,
    mut q_planner: Query<(Entity, &mut Planner)>,
) -> Result {
    for (entity, mut planner) in q_planner.iter_mut() {
        let components = local_field_components.get(entity).map_err(|_| "Didn't find any DatumComponents, make sure you called register_components with all Components you want to use with the planner")?;
        for component in components {
            planner
                .state
                .data
                .insert(component.field_key(), component.field_value());
        }
    }
    Ok(())
}

/// A formulated plan. This is created and inserted into [`Planner`] for you when you trigger [`UpdatePlan`].
#[derive(Debug, Clone, Reflect, PartialEq)]
pub struct Plan {
    /// Queue of action keys, last is current
    pub effects: Vec<Effect>,
    /// Total cost of the plan. Includes the cost of [`Self::effects`] that were already handled and thus removed.
    pub cost: usize,
    /// The goal that is to be achieved by the plan.
    pub goal: Goal,
}

/// Entity event that can be triggered on an entity that holds a [`Planner`]
/// to kickstart a new plan. If a planner is already computing a plan, the event is ignored.
#[derive(EntityEvent, Clone, Debug)]
pub struct UpdatePlan {
    /// The entity that holds the [`Planner`]
    #[event_target]
    pub planner: Entity,
    /// Goals to be achieved by the plan, ordered by priority.
    /// If `None`, [`Planner::goals`] will be used.
    pub goals: Option<Vec<Goal>>,
}

impl From<Entity> for UpdatePlan {
    fn from(entity: Entity) -> Self {
        Self {
            planner: entity,
            goals: None,
        }
    }
}

/// This observer is responsible for finding [`Planner`]s that aren't alreay computing a new plan,
/// and creates a new task for generating a new plan
pub(crate) fn create_planner_tasks(
    plan: On<UpdatePlan>,
    mut commands: Commands,
    planner: Query<&Planner, Without<PlanReceiver>>,
    names: Query<NameOrEntity, Allow<Disabled>>,
) {
    let entity = plan.planner;
    let name = names
        .get(entity)
        .map(|n| {
            if n.name.is_some() {
                format!("{entity:?}: {n}")
            } else {
                format!("{entity:?}")
            }
        })
        .unwrap_or_else(|_| format!("{entity:?}"));
    let Ok(planner) = planner.get(entity) else {
        debug!(
            "Started planner on an entity {name} that either is not a planner, is already computing a plan, or has been filtered out by a default filter. Ignoring."
        );
        return;
    };

    let state = planner.state.clone();
    let actions = planner.actions_for_dogoap.clone();
    let goals = plan.goals.clone().unwrap_or_else(|| planner.goals.clone());
    let find_plan = move || {
        goals.into_iter().find_map(|goal| {
            // This is the expensive part.
            let (nodes, cost) = make_plan(&state, &actions[..], &goal)?;
            if nodes.is_empty() {
                // This goal has realy been achieved
                None
            } else {
                let mut effects: Vec<_> = get_effects_from_plan(nodes).collect();
                // Ensure the current effect is last, so we can simply `.pop()` it
                effects.reverse();
                Some(Plan {
                    effects,
                    cost,
                    goal,
                })
            }
        })
    };

    #[cfg(feature = "compute-pool")]
    let receiver = {
        let (send, receiver) = crossbeam_channel::bounded(1);
        let future = async move {
            let plan = find_plan();
            send.send(plan).expect("Failed to send plan");
        };
        let thread_pool = AsyncComputeTaskPool::get();
        thread_pool.spawn(future).detach();
        receiver
    };
    #[cfg(not(feature = "compute-pool"))]
    let receiver = Receiver(find_plan());

    commands
        .entity(entity)
        .insert((IsPlanning, PlanReceiver(receiver)));
}

/// This system is responsible for polling active [`ComputePlan`]s and switch the `current_action` if it changed
/// since last time. It'll add the [`ActionComponent`] as a Component to the same Entity the [`Planner`] is on, and
/// remove all the others, signalling that [`Action`] is currently active.
pub(crate) fn handle_planner_tasks(
    mut commands: Commands,
    mut query: Query<(Entity, &mut PlanReceiver, &mut Planner)>,
    names: Query<NameOrEntity, Allow<Disabled>>,
) -> Result {
    #[cfg_attr(
        feature = "compute-pool",
        expect(
            unused_mut,
            reason = "The receiver doesn't need to be mutable, but we keep the code the same as in the non-compute-pool case for simplicity"
        )
    )]
    for (entity, mut task, mut planner) in query.iter_mut() {
        #[cfg(not(feature = "compute-pool"))]
        let plan = task.0.0.take();

        #[cfg(feature = "compute-pool")]
        let plan = match task.0.try_recv() {
            Ok(plan) => plan,
            Err(err) => match err {
                crossbeam_channel::TryRecvError::Empty => continue,
                crossbeam_channel::TryRecvError::Disconnected => {
                    return Err(BevyError::from("Task channel disconnected"));
                }
            },
        };

        commands.entity(entity).try_remove::<PlanReceiver>();
        match plan {
            Some(plan) => {
                planner.current_plan.replace(plan);
            }
            None => {
                let name = names
                    .get(entity)
                    .map(|n| {
                        if n.name.is_some() {
                            format!("{entity:?}: {n}")
                        } else {
                            format!("{entity:?}")
                        }
                    })
                    .unwrap_or_else(|_| format!("{entity:?}"));
                warn!("Failed to make a plan for any goal for entity {name}!");
                planner.current_action = None;
                planner.current_plan = None;
            }
        }
        commands.entity(entity).try_remove::<IsPlanning>();
    }
    Ok(())
}

pub(crate) fn execute_plan(
    mut planners: Query<(Entity, &mut Planner)>,
    planners_with_actions: Query<&dyn ActionComponent>,
    mut commands: Commands,
) {
    for (entity, mut planner) in planners.iter_mut() {
        if planners_with_actions.contains(entity) {
            // Already executing an action
            continue;
        }
        let Some(plan) = planner.current_plan.as_mut() else {
            debug!("No plan to execute");
            planner.current_action = None;
            continue;
        };
        match plan.effects.pop() {
            Some(action_name) => {
                let (found_action, action_component) =
                planner.actions_map.get(&action_name.action).unwrap_or_else(|| {
                    panic!(
                        "Didn't find action {action_name:?} registered in the Planner::actions_map"
                    )
                });

                if planner.current_action.is_some()
                    && Some(found_action) != planner.current_action.as_ref()
                {
                    // We used to work towards a different action, so lets remove that one first.
                    // action_component.remove(&mut commands, entity);
                    // WARN remove all possible actions in order to avoid race conditions for now
                    for (_, (_, component)) in planner.actions_map.iter() {
                        component.try_remove(&mut commands, entity);
                    }
                }

                action_component.try_insert(&mut commands, entity);
                planner.current_action = Some(found_action.clone());
            }
            None => {
                debug!("Current plan is finished");
                planner.current_plan = None;
                planner.current_action = None;
            }
        }
    }
}
