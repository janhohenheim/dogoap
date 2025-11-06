//! This is a basic example on how you can use Dogoap while moving your agent around

use bevy::{
    color::palettes::css::*,
    prelude::Camera2d,
    prelude::*,
    time::common_conditions::on_timer,
    window::{Window, WindowPlugin},
};
use bevy_dogoap::prelude::*;
use rand::Rng;
use std::{collections::HashMap, time::Duration};

/// This is our marker components, so we can keep track of the various in-game entities
#[derive(Component)]
struct Cell {
    speed: f32,
    age: usize,
}

#[derive(Component)]
struct DeadCell;

#[derive(Component)]
struct Food;

#[derive(Component)]
struct MoveTo(Vec3, Entity);

//
// Various actions our Cell can perform
//

// When cell is at food, the cell can consume the food, decreasing hunger
#[derive(Component, Clone, Reflect, Default, ActionComponent)]
struct EatAction;

// When we're not hungry, our cell can replicate itself
#[derive(Component, Clone, Reflect, Default, ActionComponent)]
struct ReplicateAction;

// This will make the cell seek out the closest food
#[derive(Component, Clone, Reflect, Default, ActionComponent)]
struct GoToFoodAction;

//
// All of our State fields
//

#[derive(Component, Clone, DatumComponent)]
struct Hunger(f64);

#[derive(Component, Clone, DatumComponent)]
struct AtFood(bool);

#[derive(Component, Clone, DatumComponent)]
struct IsReplicating(bool);

// UI elements
#[derive(Component)]
struct StateDebugText;

fn spawn_cell(commands: &mut Commands, position: Vec3, speed: f32) {
    let goal = Goal::from_reqs(&[IsReplicating::is(true)]);

    let eat_action = EatAction::action()
        .with_precondition(AtFood::is(true))
        .with_mutator(Hunger::decrease(10.0))
        .with_mutator(AtFood::set(true))
        .set_cost(1);

    let replicate_action = ReplicateAction::action()
        .with_precondition(Hunger::is_less(10.0))
        .with_mutator(IsReplicating::set(true))
        .with_mutator(Hunger::increase(25.0))
        .set_cost(10);

    let go_to_food_action = GoToFoodAction::action()
        .with_precondition(AtFood::is(false))
        .with_mutator(AtFood::set(true))
        .with_mutator(Hunger::increase(1.0))
        .set_cost(2);

    let mut rng = rand::rng();
    let starting_hunger = rng.random_range(20.0..45.0);

    let (planner, components) = create_planner!({
        actions: [
            (EatAction, eat_action),
            (GoToFoodAction, go_to_food_action),
            (ReplicateAction, replicate_action)
        ],
        state: [Hunger(starting_hunger), AtFood(false), IsReplicating(false)],
        goals: [goal],
    });

    let text_style = TextFont {
        font_size: 12.0,
        ..default()
    };

    commands
        .spawn((
            Name::new("Cell"),
            Cell { speed, age: 0 },
            planner,
            components,
            Transform::from_translation(position),
            GlobalTransform::from_translation(position),
            InheritedVisibility::default(),
        ))
        .with_children(|subcommands| {
            subcommands.spawn((
                Transform::from_translation(Vec3::new(10.0, -10.0, 10.0)),
                Text2d("".into()),
                text_style,
                bevy::sprite::Anchor::TOP_LEFT,
                StateDebugText,
            ));
        })
        // start an initial plan
        .trigger(UpdatePlan::from);
}

fn startup(mut commands: Commands, window: Single<&Window>) {
    let window_height = window.height() / 2.0;
    let window_width = window.width() / 2.0;

    let mut rng = rand::rng();

    for _i in 0..1 {
        let y = rng.random_range(-window_height..window_height);
        let x = rng.random_range(-window_width..window_width);
        spawn_cell(&mut commands, Vec3::from_array([x, y, 1.0]), 128.0);
    }

    // Begin with three food
    for _i in 0..30 {
        let y = rng.random_range(-window_height..window_height);
        let x = rng.random_range(-window_width..window_width);
        commands.spawn((
            Name::new("Food"),
            Food,
            Transform::from_translation(Vec3::new(x, y, 0.0)),
        ));
    }
    // Misc stuff we want somewhere
    commands.spawn(Camera2d);
}

fn spawn_random_food(
    window: Single<&Window>,
    mut commands: Commands,
    q_food: Query<Entity, With<Food>>,
) {
    let window_height = window.height() / 2.0;
    let window_width = window.width() / 2.0;

    if q_food.iter().len() < 100 {
        let mut rng = rand::rng();
        let y = rng.random_range(-window_height..window_height);
        let x = rng.random_range(-window_width..window_width);
        commands.spawn((
            Name::new("Food"),
            Food,
            Transform::from_translation(Vec3::new(x, y, 0.0)),
        ));
    }
}

fn handle_move_to(
    mut commands: Commands,
    time: Res<Time>,
    mut query: Query<(Entity, &Cell, &MoveTo, &mut Transform)>,
) {
    for (entity, cell, move_to, mut transform) in query.iter_mut() {
        let destination = move_to.0;
        let destination_entity = move_to.1;

        // Check first if destination entity exists, otherwise cancel the MoveTo,
        match commands.get_entity(destination_entity) {
            Ok(_) => {
                if transform.translation.distance(destination) > 5.0 {
                    let direction = (destination - transform.translation).normalize();
                    transform.translation += direction * cell.speed * time.delta_secs();
                } else {
                    info!("Reached destination");
                    commands.entity(entity).try_remove::<MoveTo>();
                }
            }
            Err(_) => {
                // Cancel the MoveTo order as the destination no longer exists...
                commands.entity(entity).try_remove::<MoveTo>();
            }
        }
    }
}

fn handle_go_to_food_action(
    mut commands: Commands,
    mut query: Query<
        (Entity, &Transform, &mut AtFood),
        (With<GoToFoodAction>, Without<Food>, Without<MoveTo>),
    >,
    q_food: Query<(Entity, &Transform), With<Food>>,
    mut targeted_food: Local<HashMap<Entity, Entity>>,
) {
    for (entity, t_entity, mut at_food) in query.iter_mut() {
        let origin = t_entity.translation;
        let items: Vec<(Entity, Transform)> = q_food.iter().map(|(e, t)| (e, *t)).collect();

        let foods = find_closest(origin, items);

        let mut selected_food = None;
        for (e_food, t_food, distance) in foods.iter() {
            match targeted_food.get(e_food) {
                Some(cell_entity) if *cell_entity == entity => {
                    // This food is targeted by us, select it
                    selected_food = Some((e_food, t_food, distance));
                    break;
                }
                Some(_) => {
                    // This food is targeted by another entity, skip it
                    continue;
                }
                None => {
                    // This food is not targeted, select it
                    selected_food = Some((e_food, t_food, distance));
                    break;
                }
            }
        }
        let Some((e_food, t_food, distance)) = selected_food else {
            // No available food found, do nothing
            continue;
        };

        targeted_food.insert(*e_food, entity);

        if *distance > 5.0 {
            commands.entity(entity).insert(MoveTo(*t_food, *e_food));
        } else {
            // Consume food!
            info!("Consumed food");
            at_food.0 = true;
            commands.entity(entity).remove::<GoToFoodAction>();
            targeted_food.remove(e_food);
        }
    }
}

fn find_closest(origin: Vec3, items: Vec<(Entity, Transform)>) -> Vec<(Entity, Vec3, f32)> {
    let mut closest: Vec<(Entity, Vec3, f32)> = items
        .into_iter()
        .map(|(entity, transform)| {
            let distance = transform.translation.distance(origin);
            (entity, transform.translation, distance)
        })
        .collect();

    closest.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));
    closest.truncate(10);
    closest
}

fn handle_replicate_action(
    mut commands: Commands,
    mut query: Query<(Entity, &mut Hunger, &Cell, &Transform), With<ReplicateAction>>,
    mut timers: Local<HashMap<Entity, Timer>>,
    time: Res<Time>,
) {
    for (entity, mut hunger, cell, transform) in query.iter_mut() {
        match timers.get_mut(&entity) {
            Some(progress) => {
                if progress.tick(time.delta()).just_finished() {
                    let new_transform = transform.translation + Vec3::from_array([25., 0., 0.]);
                    spawn_cell(&mut commands, new_transform, cell.speed);
                    commands.entity(entity).remove::<ReplicateAction>();
                    hunger.0 += 20.0;
                    timers.remove(&entity);
                    commands.entity(entity).trigger(UpdatePlan::from);
                } else {
                    hunger.0 += 6.0 * time.delta_secs_f64();
                }
            }
            None => {
                timers.insert(entity, Timer::from_seconds(3.0, TimerMode::Once));
            }
        }
    }
}

fn handle_eat_action(
    mut commands: Commands,
    mut query: Query<
        (Entity, &Transform, &mut Hunger, &mut AtFood),
        (With<EatAction>, Without<Food>),
    >,
    q_food: Query<(Entity, &Transform), With<Food>>,
) {
    for (entity, t_entity, mut hunger, mut at_food) in query.iter_mut() {
        let origin = t_entity.translation;
        let items: Vec<(Entity, Transform)> = q_food.iter().map(|(e, t)| (e, *t)).collect();
        let foods = find_closest(origin, items);
        let food = foods.first();

        let Some((e_food, _t_food, distance)) = food else {
            panic!("No food could be found, HOW?!")
        };

        // Make sure we're actually in range to consume this food
        // If not, remove the EatAction to cancel it, and the planner
        // will figure out what to do next
        if *distance < 5.0 {
            // Before we consume this food, make another query to ensure
            // it's still there, as it could have been consumed by another
            // Cell in the same frame, during the query.iter() loop
            if q_food.contains(*e_food) {
                hunger.0 -= 10.0;

                if hunger.0 < 0.0 {
                    hunger.0 = 0.0;
                }
                commands.entity(*e_food).despawn();
            } else {
                // Don't consume as it doesn't exists
                warn!("Tried to consume non-existing food");
            }
        }

        commands.entity(entity).remove::<EatAction>();
        at_food.0 = false;
    }
}

fn print_cell_count(query: Query<Entity, With<Cell>>) {
    info!("Active Cells: {}", query.iter().len());
}

fn over_time_needs_change(
    mut commands: Commands,
    time: Res<Time>,
    mut query: Query<(Entity, &mut Hunger, &Transform)>,
) {
    let mut rng = rand::rng();
    for (entity, mut hunger, transform) in query.iter_mut() {
        // Increase hunger
        let r = rng.random_range(10.0..20.0);
        let val: f64 = r * time.delta_secs_f64();
        hunger.0 += val;
        if hunger.0 > 100.0 {
            commands.entity(entity).despawn();
            let translation = transform.translation;
            commands.spawn((
                DeadCell,
                Transform::from_translation(translation),
                GlobalTransform::from_translation(translation),
            ));
            info!("Removed starving Cell");
        }
    }
}

fn print_current_local_state(
    query: Query<(Entity, &Cell, &Hunger, &Children)>,
    q_actions: Query<(
        Option<&IsPlanning>,
        Option<&EatAction>,
        Option<&GoToFoodAction>,
        Option<&ReplicateAction>,
    )>,
    q_child: Query<Entity, With<StateDebugText>>,
    mut text_writer: Text2dWriter,
) {
    for (entity, cell, hunger, children) in query.iter() {
        let age = cell.age;
        let hunger = hunger.0;

        let mut current_action = "Idle";

        let (is_planning, eat, go_to_food, replicate) = q_actions.get(entity).unwrap();

        if is_planning.is_some() {
            current_action = "Planning...";
        }

        if eat.is_some() {
            current_action = "Eating";
        }

        if go_to_food.is_some() {
            current_action = "Going to food";
        }

        if replicate.is_some() {
            current_action = "Replicating";
        }

        for child in children.iter() {
            let text = q_child.get(child).unwrap();
            *text_writer.text(text, 0) =
                format!("{current_action}\nAge: {age}\nHunger: {hunger:.0}\nEntity: {entity}");
        }
    }
}

// Worlds shittiest graphics incoming, beware and don't copy
fn draw_gizmos(
    mut gizmos: Gizmos,
    q_cell: Query<(&Transform, &Cell)>,
    q_dead: Query<&Transform, With<DeadCell>>,
    q_food: Query<&Transform, With<Food>>,
) {
    gizmos
        .grid_2d(
            Vec2::ZERO,
            UVec2::new(16, 9),
            Vec2::new(80., 80.),
            // Dark gray
            Srgba::new(0.1, 0.1, 0.1, 0.5),
        )
        .outer_edges();

    for (cell_transform, cell) in q_cell.iter() {
        let color = NAVY;
        color.lighter((cell.age / 100) as f32);
        gizmos.circle_2d(cell_transform.translation.truncate(), 12., color);
    }

    for food_transform in q_food.iter() {
        gizmos.circle_2d(food_transform.translation.truncate(), 4., GREEN_YELLOW);
    }

    for cell_transform in q_dead.iter() {
        gizmos.circle_2d(
            cell_transform.translation.truncate(),
            12.,
            Srgba::new(1.0, 0.0, 0.0, 0.1),
        );
    }
}

fn increment_age(mut query: Query<&mut Cell>) {
    for mut cell in query.iter_mut() {
        cell.age += 1;
    }
}

fn main() {
    let mut app = App::new();

    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            canvas: Some("#example-canvas".into()),
            ..default()
        }),
        ..default()
    }))
    .add_plugins(DogoapPlugin::default())
    .add_systems(Startup, startup)
    .add_systems(Update, draw_gizmos)
    .add_systems(
        FixedUpdate,
        (
            handle_move_to,
            handle_go_to_food_action,
            handle_eat_action,
            handle_replicate_action,
        )
            .chain(),
    )
    .add_systems(
        FixedUpdate,
        (
            spawn_random_food.run_if(on_timer(Duration::from_millis(100))),
            over_time_needs_change.run_if(on_timer(Duration::from_millis(100))),
            print_cell_count.run_if(on_timer(Duration::from_millis(1000))),
            increment_age.run_if(on_timer(Duration::from_millis(1000))),
            print_current_local_state.run_if(on_timer(Duration::from_millis(50))),
        ),
    );

    register_components!(app, [Hunger, AtFood]);
    register_actions!(app, [EatAction, GoToFoodAction, ReplicateAction]);

    app.run();
}
