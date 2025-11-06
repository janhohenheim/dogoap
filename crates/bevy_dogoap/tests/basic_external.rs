//! Tests the basics using the external API

use bevy::prelude::*;
use bevy_dogoap::prelude::*;

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
    // Then we decide a goal of not being hungry nor tired
    let goal = Goal::from_reqs(&[IsHungry::is(false), IsTired::is(false)]);

    // Alternatively, the `simple` functions can help you create things a bit smoother
    let eat_action = EatAction::action()
        .with_precondition(IsTired::is(false))
        .with_mutator(IsHungry::set(false));

    // Here we define our SleepAction
    let sleep_action = SleepAction::action().with_mutator(IsTired::set(false));

    // But we have a handy macro that kind of makes it a lot easier for us!
    let (planner, components) = create_planner!({
        actions: [
            (EatAction, eat_action),
            (SleepAction, sleep_action),
        ],
        state: [IsHungry(true), IsTired(true)],
        goals: [goal],
    });

    commands
        .spawn((planner, components))
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
        is_tired.0 = false;
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
    fn get_state(app: &mut App) -> LocalState {
        let mut query = app.world_mut().query::<&Planner>();
        let planners: Vec<&Planner> = query.iter(app.world()).collect();

        let planner = planners.first().unwrap();

        planner.state.clone()
    }

    fn assert_key_is_bool(app: &mut App, key: &str, expected_bool: bool, msg: &str) {
        let state = get_state(app);
        let expected_val = Datum::Bool(expected_bool);
        let found_val = state.data.get(key).unwrap();
        assert_eq!(*found_val, expected_val, "{msg}");
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
    fn test_basic_bevy_integration_external() {
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
        .add_systems(Startup, startup)
        .add_systems(
            FixedUpdate,
            (start_new_plan, handle_eat_action, handle_sleep_action),
        )
        .add_observer(|_: On<Remove, IsPlanning>, mut commands: Commands| {
            commands.insert_resource(PlannerDone);
        });

        register_components!(app, [IsHungry, IsTired]);
        register_actions!(app, [EatAction, SleepAction]);

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

        info!("Final State:\n{:#?}", get_state(&mut app));

        assert_key_is_bool(&mut app, "is_hungry", false, "is_hungry wasn't false");
        assert_key_is_bool(&mut app, "is_tired", false, "is_tired wasn't false");
        assert_component_not_exists::<EatAction>(&mut app);
        assert_component_not_exists::<SleepAction>(&mut app);

        info!("Final State:\n{:#?}", get_state(&mut app));
    }
}
