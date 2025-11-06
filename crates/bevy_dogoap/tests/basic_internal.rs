//! Tests the basics using the internal API

use bevy::prelude::*;
use bevy_dogoap::prelude::*;
use bevy_platform::collections::HashMap;
use dogoap::simple::simple_action;

// These are just handy strings so we don't fuck it up later.
const IS_HUNGRY_KEY: &str = "is_hungry";
const EAT_ACTION: &str = "eat_action";

const IS_TIRED_KEY: &str = "is_tired";
const SLEEP_ACTION: &str = "sleep_action";

// This is our component we want to be able to use
#[derive(Component, Clone, Reflect, Default, ActionComponent)]
struct EatAction;

#[derive(Component, Clone, Reflect, Default, ActionComponent)]
struct SleepAction;

#[derive(Component, Clone, DatumComponent)]
struct IsHungry(bool);

#[derive(Component, Clone, DatumComponent)]
struct IsTired(bool);

fn startup(mut commands: Commands) {
    // First we define our initial state
    // verbose way:
    // let state = LocalState::new()
    //     .with_field(IS_HUNGRY_KEY, Field::from(true))
    //     .with_field(IS_TIRED_KEY, Field::from(true));
    let components = vec![
        Box::new(IsHungry(true)) as Box<dyn DatumComponent>,
        Box::new(IsTired(true)) as Box<dyn DatumComponent>,
    ];

    // Then we decide a goal of not being hungry nor tired
    let goal = Goal::new()
        .with_req(IS_HUNGRY_KEY, Compare::equals(false))
        .with_req(IS_TIRED_KEY, Compare::equals(false));

    // All goals our planner could use
    let goals = vec![goal.clone()];

    // The verbose way of defining our action
    // let eat_action = Action {
    //     key: EAT_ACTION.to_string(),
    //     // We need to not be tired in order to eat
    //     preconditions: Some(vec![(
    //         IS_TIRED_KEY.to_string(),
    //         Compare::Equals(Field::from(false)),
    //     )]),
    //     options: vec![(
    //         Effect {
    //             action: EAT_ACTION.to_string(),
    //             argument: None,
    //             // The "Effect" of our EatAction is that "is_hungry" gets set to "false"
    //             mutators: vec![Mutator::Set(
    //                 IS_HUNGRY_KEY.to_string(),
    //                 Field::Bool(false),
    //             )],
    //             state: DogoapState::new(),
    //         },
    //         1,
    //     )],
    // };

    // Alternatively, the `simple` functions can help you create things a bit smoother
    let eat_action = simple_action(EAT_ACTION, IS_HUNGRY_KEY, false)
        .with_precondition((IS_TIRED_KEY, Compare::equals(false)));

    // Here we define our SleepAction
    let sleep_action = simple_action(SLEEP_ACTION, IS_TIRED_KEY, false);

    // Verbose way of defining an actions_map that the planner needs
    let actions_map = HashMap::from([
        (
            EAT_ACTION.to_string(),
            (
                eat_action.clone(),
                Box::new(EatAction) as Box<dyn InserterComponent>,
            ),
        ),
        (
            SLEEP_ACTION.to_string(),
            (
                sleep_action.clone(),
                Box::new(SleepAction) as Box<dyn InserterComponent>,
            ),
        ),
    ]);

    let entity = commands.spawn_empty().id();

    let planner = Planner::new(components, goals, actions_map);

    commands
        .entity(entity)
        .insert((planner, (IsHungry(true), IsTired(true))))
        .trigger(UpdatePlan::from);
}

fn start_new_plan(mut commands: Commands, planner: Query<Entity, With<Planner>>) {
    for planner in planner.iter() {
        commands.entity(planner).trigger(UpdatePlan::from);
    }
}

fn handle_eat_action(
    mut commands: Commands,
    mut query: Query<(Entity, &mut IsHungry), With<EatAction>>,
) {
    for (entity, mut is_hungry) in query.iter_mut() {
        info!("We're doing EatAction!");
        is_hungry.0 = false;
        // planner
        //     .state
        //     .fields
        //     .insert(IS_HUNGRY_KEY.to_string(), Field::Bool(false));
        commands.entity(entity).remove::<EatAction>();
        info!("Removed EatAction from our Entity {entity}");
    }
}

fn handle_sleep_action(
    mut commands: Commands,
    mut query: Query<(Entity, &mut IsTired), With<SleepAction>>,
) {
    for (entity, mut is_tired) in query.iter_mut() {
        info!("We're doing SleepAction!");
        // *is_tired = IsTired(false);
        is_tired.0 = false;
        // planner
        //     .state
        //     .fields
        //     .insert(IS_TIRED_KEY.to_string(), Field::Bool(false));
        commands.entity(entity).remove::<SleepAction>();
        info!("Removed SleepAction from our Entity {entity}");
    }
}

mod test {
    use std::time::Duration;

    use bevy::time::TimeUpdateStrategy;
    use bevy_log::LogPlugin;

    use super::*;

    // Test utils
    fn get_planner(app: &mut App) -> &Planner {
        let mut query = app.world_mut().query::<&Planner>();
        let planners: Vec<&Planner> = query.iter(app.world()).collect();

        planners.first().unwrap()
    }

    fn get_state(app: &mut App) -> LocalState {
        let planner = get_planner(app);
        // planner.field_components_to_localstate()
        planner.state.clone()
    }

    fn assert_key_is_bool(app: &mut App, key: &str, expected_bool: bool) {
        let state = get_state(app);
        let expected_val = Datum::Bool(expected_bool);
        let found_val = state.data.get(key).unwrap();
        assert_eq!(*found_val, expected_val);
    }

    fn assert_component_not_exists<T>(app: &mut App)
    where
        T: Component,
    {
        let mut query = app.world_mut().query::<&T>();
        let c = query.iter(app.world()).len();
        assert!(c == 0);
    }

    #[test]
    fn test_basic_bevy_integration_internal() {
        let mut app = App::new();

        #[derive(Resource)]
        struct PlannerDone;

        app.add_plugins((
            MinimalPlugins,
            LogPlugin::default(),
            DogoapPlugin::default(),
        ))
        .insert_resource(TimeUpdateStrategy::ManualDuration(
            // make every `app.update()` trigger a fixed loop
            Time::<Fixed>::default().timestep(),
        ))
        .register_component_as::<dyn DatumComponent, IsHungry>()
        .register_component_as::<dyn DatumComponent, IsTired>()
        .register_component_as::<dyn ActionComponent, EatAction>()
        .register_component_as::<dyn ActionComponent, SleepAction>()
        .add_systems(Startup, startup)
        .add_systems(
            FixedUpdate,
            (start_new_plan, handle_eat_action, handle_sleep_action),
        )
        .add_observer(|_: On<Remove, IsPlanning>, mut commands: Commands| {
            commands.insert_resource(PlannerDone);
        });

        app.finish();

        // Spin until the planner is done planning
        loop {
            // sleep because we're waiting for another thread to be done
            std::thread::sleep(Duration::from_millis(50));
            app.update();
            if app.world_mut().get_resource::<PlannerDone>().is_some() {
                break;
            }
        }

        // Execute the plan
        for _ in 0..4 {
            app.update();
        }

        assert_key_is_bool(&mut app, IS_HUNGRY_KEY, false);
        assert_key_is_bool(&mut app, IS_TIRED_KEY, false);
        assert_component_not_exists::<EatAction>(&mut app);
        assert_component_not_exists::<SleepAction>(&mut app);

        info!("Final State:\n{:#?}", get_state(&mut app));
    }
}
