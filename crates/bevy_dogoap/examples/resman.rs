//! Lemonade Stand / Restaurant Simulation
//!
//! Purpose
//! - A minimal, end-to-end example of using bevy_dogoap to drive agents via goals, precondition/effect actions, sensing, and reactive replanning.
//!
//! - Model planner state as datums (components) like `Thirst`, `AtOrderDesk`, `HasPendingOrder`.
//! - Define actions with preconditions and effects (e.g., `PlaceOrder`, `ProduceLemonade`, `ServeOrder`).
//! - Sense/derive facts each frame and pick goals; replan when conditions change.
//! - Coordinate multi-actor steps with simple desk “sessions” (order + serve) using timers.
//!
//! World
//! - Customers: thirst increases; when above a threshold, they plan to drink. They go to the
//!   desk, place an order, wait, pick up, drink, then wander.
//! - Worker: reacts to demand (or a call-to-desk), takes the order, produces at the maker,
//!   returns and serves. Has energy; rests at a chair when low.
//! - Business: money increases on serve; simple UI + debug text visualize state.
//!
//! Patterns used
//! - Multi-actor sessions: OrderSession/ServeSession coordinate customers and worker, requiring
//!   both actors present at the desk. Timers gate completion; if either leaves, session cancels.
//! - Reactive goals: thirst/energy drift continuously, goals activate/clear based on thresholds
//!   rather than discrete events. Triggers replanning when crossed.
//! - Idle behavior: when goals clear (thirst satisfied, no work), agents wander rather than freeze.
//! - Call semantics: customers trigger ShouldGoToOrderDesk when placing orders, pulling the
//!   worker to the desk reactively rather than polling.
//! - Activity-dependent state: energy decay multiplier varies (producing vs idle), modeling
//!   realistic effort costs.
//! - Atomic session updates: all state changes (spawn order, transfer item, pay money) happen
//!   together when session timer completes, avoiding partial states.
//! - Schedule: FixedPreUpdate derives facts and sets goals → planner runs → FixedUpdate executes
//!   movement/actions and sessions → facts re-derived → invariants/UI.

use std::collections::VecDeque;

use bevy::{
    color::palettes::css::*,
    prelude::*,
    window::{Window, WindowPlugin},
};
use bevy_dogoap::plugin::DogoapSystems;
use bevy_dogoap::prelude::*;
use rand::Rng;

const THIRST_RATE: f64 = 0.2;
const THIRST_THRESHOLD: f64 = 6.0; // when to trigger drinking behaviour

const MOVE_SPEED: f32 = 96.0;
const ARRIVAL_RADIUS: f32 = 5.0; // px

const DRINK_TIME: f32 = 1.0;
const PRODUCE_TIME: f32 = 1.2;
const SERVE_TIME: f32 = 0.7;
const ORDERING_TIME: f32 = 1.0; // joint PlaceOrder/TakeOrder session duration

// Worker Energy
const ENERGY_LOW_THRESH: f64 = 0.1; // trigger rest
const ENERGY_TARGET: f64 = 0.8; // target to stop resting
const ENERGY_DECAY_RATE: f64 = 0.04; // per second when not resting
const ENERGY_GAIN_RATE: f64 = 0.08; // per second when at chair
const ENERGY_DECAY_PRODUCE_MULT: f64 = 1.6; // when producing lemonade, decay multiplier

// Customer wander area (around spawn area)
const WANDER_CENTER_X: f32 = -220.0; // between the two customer spawns
const WANDER_CENTER_Y: f32 = -100.0; // spawn y
const WANDER_HALF_WIDTH: f32 = 60.0; // total width ~120
const WANDER_HALF_HEIGHT: f32 = 40.0; // total height ~80
const WANDER_MIN_X: f32 = WANDER_CENTER_X - WANDER_HALF_WIDTH;
const WANDER_MAX_X: f32 = WANDER_CENTER_X + WANDER_HALF_WIDTH;
const WANDER_MIN_Y: f32 = WANDER_CENTER_Y - WANDER_HALF_HEIGHT;
const WANDER_MAX_Y: f32 = WANDER_CENTER_Y + WANDER_HALF_HEIGHT;

// Visual layout offsets
const DESK_CUSTOMER_OFFSET: Vec3 = Vec3::new(-50.0, 0.0, 0.0);
const DESK_WORKER_OFFSET: Vec3 = Vec3::new(50.0, 0.0, 0.0);

#[derive(Resource, Default, Debug, Clone, Copy)]
struct Money(i64);

// Markers

#[derive(Component)]
struct Agent;

#[derive(Component, Default)]
struct Customer {
    order: Option<Entity>,
}

#[derive(Component)]
struct Worker;

#[derive(Component)]
struct LemonadeMaker;

#[derive(Component)]
struct Chair;

#[derive(Component, Default)]
struct OrderDesk {
    // Derived each frame from presence and occupancy
    can_take_order: bool,
    current_order: Option<Entity>,
}

#[derive(Component, Default)]
struct OrderSession {
    customer: Option<Entity>,
    worker: Option<Entity>,
    timer: Option<Timer>,
}

#[derive(Component, Default)]
struct ServeSession {
    customer: Option<Entity>,
    worker: Option<Entity>,
    timer: Option<Timer>,
}

#[derive(Component)]
struct Order {
    items_to_produce: VecDeque<Item>,
}

#[derive(Clone, Default, Copy, Reflect, Debug, PartialEq, Eq)]
enum Item {
    #[default]
    Nothing,
    Lemonade,
}

#[derive(Component)]
struct StateDebugText;

#[derive(Component)]
struct MoneyText;

#[derive(Component)]
struct MoveTo(Vec3);

#[derive(Component)]
struct ActionProgress(Timer);

#[derive(Component)]
struct IdleWanderTimer(Timer);

// Small helpers to de-duplicate common patterns

fn random_wander_target() -> Vec3 {
    let mut rng = rand::rng();
    let rx = rng.random_range(WANDER_MIN_X..WANDER_MAX_X);
    let ry = rng.random_range(WANDER_MIN_Y..WANDER_MAX_Y);
    Vec3::new(rx, ry, 0.0)
}

fn move_or_arrive(commands: &mut Commands, e: Entity, t: &Transform, dest: Vec3) -> bool {
    if t.translation.distance(dest) > ARRIVAL_RADIUS {
        commands.entity(e).insert(MoveTo(dest));
        false
    } else {
        true
    }
}

fn progress_or_start(
    commands: &mut Commands,
    e: Entity,
    progress: Option<Mut<ActionProgress>>,
    secs: f32,
    time: &Time,
) -> bool {
    if let Some(mut prog) = progress {
        if prog.0.tick(time.delta()).just_finished() {
            true
        } else {
            false
        }
    } else {
        commands
            .entity(e)
            .insert(ActionProgress(Timer::from_seconds(secs, TimerMode::Once)));
        false
    }
}

fn set_goals_and_replan(
    commands: &mut Commands,
    e: Entity,
    planner: &mut Planner,
    goals: Vec<Goal>,
) {
    if planner.goals.as_slice() != goals.as_slice() {
        planner.goals = goals;
        planner.current_plan = None;
        planner.current_action = None;
        commands.entity(e).trigger(UpdatePlan::from);
    }
}

fn has_any_action(e: Entity, q_actions: &Query<(Entity, &dyn ActionComponent)>) -> bool {
    for (_ent, actions) in q_actions.get(e).iter() {
        if actions.iter().next().is_some() {
            return true;
        }
    }
    false
}

fn desk_pos_for_customer(desk_t: &Transform) -> Vec3 {
    desk_t.translation + DESK_CUSTOMER_OFFSET
}

fn desk_pos_for_worker(desk_t: &Transform) -> Vec3 {
    desk_t.translation + DESK_WORKER_OFFSET
}

fn remove_if<C: Component>(commands: &mut Commands, e: Entity, cond: bool) {
    if cond {
        commands.entity(e).remove::<C>();
    }
}

fn session_step(time: &Time, timer: &mut Option<Timer>, both_present: bool, start_secs: Option<f32>) -> bool {
    if timer.is_some() && !both_present {
        *timer = None;
        return false;
    }
    if timer.is_none() {
        if let Some(secs) = start_secs {
            *timer = Some(Timer::from_seconds(secs, TimerMode::Once));
        }
    }
    if let Some(t) = timer.as_mut() {
        if t.tick(time.delta()).just_finished() {
            *timer = None;
            return true;
        }
    }
    false
}

fn spawn_labeled<T: Component>(
    commands: &mut Commands,
    name: &str,
    marker: T,
    pos: Vec3,
    label: &str,
    label_offset: Vec3,
) {
    commands
        .spawn((
            Name::new(name.to_string()),
            marker,
            InheritedVisibility::default(),
            Transform::from_translation(pos),
        )).with_children(|sub| {
            sub.spawn((
                Transform::from_translation(label_offset),
                Text2d(label.into()),
                TextFont { font_size: 12.0, ..default() },
                bevy::sprite::Anchor::TOP_LEFT,
            ));
        });
}

// Datums ("state fields")

#[derive(Component, Clone, DatumComponent)]
struct Thirst(f64);

#[derive(Component, Clone, EnumComponent)]
struct CarryingItem(Item);

#[derive(Component, Clone, DatumComponent)]
struct PlacedOrder(bool);

#[derive(Component, Clone, DatumComponent)]
struct OrderReady(bool);

#[derive(Component, Clone, DatumComponent)]
struct AtOrderDesk(bool);

#[derive(Component, Clone, DatumComponent)]
struct AtLemonadeMaker(bool);

#[derive(Component, Clone, DatumComponent)]
struct AtChair(bool);

#[derive(Component, Clone, DatumComponent)]
struct ShouldGoToOrderDesk(bool);

#[derive(Component, Clone, DatumComponent)]
struct HasPendingOrder(bool);

#[derive(Component, Clone, DatumComponent)]
struct ServedOrder(bool);

#[derive(Component, Clone, DatumComponent)]
struct OrderTaken(bool);

#[derive(Component, Clone, DatumComponent)]
struct Energy(f64);

// Actions

#[derive(Component, Clone, Default, ActionComponent)]
struct DrinkLemonade;

#[derive(Component, Clone, Default, ActionComponent)]
struct PickupOrder;

#[derive(Component, Clone, Default, ActionComponent)]
struct WaitForOrder;

#[derive(Component, Clone, Default, ActionComponent)]
struct PlaceOrder;

#[derive(Component, Clone, Default, ActionComponent)]
struct GoToOrderDesk;

#[derive(Component, Clone, Default, ActionComponent)]
struct GoToLemonadeMaker;

#[derive(Component, Clone, Default, ActionComponent)]
struct ProduceLemonade;

#[derive(Component, Clone, Default, ActionComponent)]
struct ServeOrder;

#[derive(Component, Clone, Default, ActionComponent)]
struct Rest;

#[derive(Component, Clone, Default, ActionComponent)]
struct TakeOrder;

#[derive(Component, Clone, Default, ActionComponent)]
struct GoToChair;

// App entry and setup

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
enum ExampleSystems {
    Exec,
    PostSense,
    Invariants,
}

fn main() {
    let mut app = App::new();

    register_components!(
        app,
        [
            Thirst,
            CarryingItem,
            PlacedOrder,
            OrderReady,
            AtOrderDesk,
            AtLemonadeMaker,
            AtChair,
            ShouldGoToOrderDesk,
            HasPendingOrder,
            ServedOrder,
            OrderTaken,
            Energy
        ]
    );
    register_actions!(
        app,
        [
            DrinkLemonade,
            PickupOrder,
            WaitForOrder,
            PlaceOrder,
            GoToOrderDesk,
            GoToLemonadeMaker,
            GoToChair,
            TakeOrder,
            ProduceLemonade,
            ServeOrder,
            Rest
        ]
    );

    app.insert_resource(Money(0))
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                canvas: Some("#example-canvas".into()),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(DogoapPlugin::default())
        .add_systems(Startup, setup)
        .add_systems(Update, (draw_state_debug, draw_ui, update_money_text))
        // Sense + Goals: happen before planner runs (FixedPreUpdate)
        .add_systems(
            FixedPreUpdate,
            (
                derive_at_stations,
                derive_has_pending_order,
                derive_can_take_order,
                update_energy,
                update_thirst,
                update_customer_goal,
                update_worker_goal,
                trigger_replanning,
            )
                .chain()
                .before(DogoapSystems::RunPlanner),
        )
        // Execution & domain handlers in FixedUpdate
        .configure_sets(
            FixedUpdate,
            (
                ExampleSystems::Exec,
                ExampleSystems::PostSense,
                ExampleSystems::Invariants,
            )
                .chain(),
        )
        .add_systems(
            FixedUpdate,
            (
                handle_move_to,
                handle_go_to_order_desk,
                handle_go_to_lemonade_maker,
                handle_go_to_chair,
                handle_customer_wander,
                handle_order_session,
                handle_take_order,
                handle_wait_for_order,
                handle_produce_lemonade,
                handle_serve_session,
                handle_serve_order,
                handle_pickup_order,
                handle_drink_lemonade,
                handle_rest,
                call_worker_to_desk,
            )
                .in_set(ExampleSystems::Exec),
        )
        // Re-derive post-exec so invariants and next-frame logic see fresh facts
        .add_systems(
            FixedUpdate,
            (derive_at_stations, derive_has_pending_order, derive_can_take_order)
                .in_set(ExampleSystems::PostSense),
        )
        .add_systems(FixedUpdate, check_invariants.in_set(ExampleSystems::Invariants))
        .run();
}

fn setup(mut commands: Commands) {
    // Customers
    spawn_customer(&mut commands, "Customer 1", 12.0, Vec3::new(-200.0, -100.0, 1.0));
    spawn_customer(&mut commands, "Customer 2", 0.0, Vec3::new(-240.0, -100.0, 1.0));

    // Workers
    spawn_worker(&mut commands, Vec3::new(140.0, -100.0, 1.0));

    // Stations
    spawn_labeled(
        &mut commands,
        "LemonadeMaker",
        LemonadeMaker,
        Vec3::new(170.0, 0.0, 1.0),
        "Lemonade Maker",
        Vec3::new(0.0, 30.0, 10.0),
    );

    commands
        .spawn((
            Name::new("OrderDesk"),
            OrderDesk::default(),
            OrderSession::default(),
            ServeSession::default(),
            InheritedVisibility::default(),
            Transform::from_xyz(-100.0, 0.0, 1.0),
        ))
        .with_children(|sub| {
            sub.spawn((
                Transform::from_translation(Vec3::new(0.0, 50.0, 10.0)),
                Text2d("Order Desk".into()),
                TextFont { font_size: 12.0, ..default() },
                bevy::sprite::Anchor::TOP_LEFT,
            ));
        });

    // Chair
    spawn_labeled(
        &mut commands,
        "Chair",
        Chair,
        Vec3::new(70.0, -120.0, 1.0),
        "Chair",
        Vec3::new(0.0, 20.0, 10.0),
    );

    // Camera
    commands.spawn(Camera2d);

    // Money UI
    commands.spawn((
        Transform::from_translation(Vec3::new(-360.0, 220.0, 100.0)),
        Text2d("Money: 0".into()),
        TextFont { font_size: 16.0, ..default() },
        bevy::sprite::Anchor::TOP_LEFT,
        MoneyText,
    ));
}

fn spawn_customer(commands: &mut Commands, name: &str, thirst_initial: f64, pos: Vec3) {
    let customer_goal = Goal::from_reqs(&[Thirst::is_less(THIRST_THRESHOLD)]);

    let drink = DrinkLemonade::action()
        .with_precondition(CarryingItem::is(Item::Lemonade))
        .with_mutator(CarryingItem::set(Item::Nothing))
        .with_mutator(Thirst::decrease(10.0));

    let pickup = PickupOrder::action()
        .with_precondition(CarryingItem::is(Item::Nothing))
        .with_precondition(OrderReady::is(true))
        .with_precondition(AtOrderDesk::is(true))
        .with_mutator(CarryingItem::set(Item::Lemonade))
        .with_mutator(PlacedOrder::set(false))
        .with_mutator(OrderReady::set(false))
        .with_mutator(AtOrderDesk::set(false));

    let wait = WaitForOrder::action()
        .with_precondition(PlacedOrder::is(true))
        .with_precondition(OrderReady::is(false))
        .with_precondition(AtOrderDesk::is(true))
        .with_mutator(OrderReady::set(true));

    let place = PlaceOrder::action()
        .with_precondition(PlacedOrder::is(false))
        .with_precondition(CarryingItem::is(Item::Nothing))
        .with_precondition(AtOrderDesk::is(true))
        .with_mutator(PlacedOrder::set(true));

    let go_to_desk = GoToOrderDesk::action()
        .with_precondition(AtOrderDesk::is(false))
        .with_mutator(AtOrderDesk::set(true));

    let (planner, state) = create_planner!({
        actions: [
            (DrinkLemonade, drink),
            (PickupOrder, pickup),
            (WaitForOrder, wait),
            (PlaceOrder, place),
            (GoToOrderDesk, go_to_desk),
        ],
        state: [
            Thirst(thirst_initial),
            CarryingItem(Item::Nothing),
            PlacedOrder(false),
            OrderReady(false),
            AtOrderDesk(false),
            AtLemonadeMaker(false),
        ],
        goals: [customer_goal],
    });

    commands
        .spawn((
            Agent,
            Name::new(name.to_string()),
            Customer::default(),
            InheritedVisibility::default(),
            planner,
            state,
            Transform::from_translation(pos),
        ))
        .with_children(|sub| {
            sub.spawn((
                Transform::from_translation(Vec3::new(-70.0, 0.0, 10.0)),
                Text2d("".into()),
                TextFont { font_size: 12.0, ..default() },
                bevy::sprite::Anchor::TOP_LEFT,
                StateDebugText,
            ));
        });
}

fn spawn_worker(commands: &mut Commands, pos: Vec3) {
    let serve = ServeOrder::action()
        .with_precondition(CarryingItem::is(Item::Lemonade))
        .with_precondition(AtOrderDesk::is(true))
        .with_mutator(CarryingItem::set(Item::Nothing))
        .with_mutator(ServedOrder::set(true))
        .with_mutator(HasPendingOrder::set(false));

    let produce = ProduceLemonade::action()
        .with_precondition(HasPendingOrder::is(true))
        .with_precondition(OrderTaken::is(true))
        .with_precondition(AtLemonadeMaker::is(true))
        .with_mutator(CarryingItem::set(Item::Lemonade));

    let go_to_maker = GoToLemonadeMaker::action()
        .with_precondition(HasPendingOrder::is(true))
        .with_precondition(OrderTaken::is(true))
        .with_precondition(AtLemonadeMaker::is(false))
        .with_mutator(AtLemonadeMaker::set(true))
        .with_mutator(AtOrderDesk::set(false));

    let take_order = TakeOrder::action()
        .with_precondition(AtOrderDesk::is(true))
        .with_precondition(CarryingItem::is(Item::Nothing))
        .with_precondition(OrderTaken::is(false))
        .with_mutator(OrderTaken::set(true))
        .with_mutator(ShouldGoToOrderDesk::set(false));

    let go_to_desk = GoToOrderDesk::action()
        .with_precondition(AtOrderDesk::is(false))
        .with_mutator(AtOrderDesk::set(true))
        .with_mutator(AtLemonadeMaker::set(false))
        .with_mutator(ShouldGoToOrderDesk::set(false));

    let rest = Rest::action()
        .with_precondition(AtChair::is(true))
        .with_mutator(Energy::increase(1.0));

    let go_to_chair = GoToChair::action()
        .with_precondition(AtChair::is(false))
        .with_mutator(AtChair::set(true))
        .with_mutator(AtOrderDesk::set(false))
        .with_mutator(AtLemonadeMaker::set(false));

    let worker_goal = Goal::from_reqs(&[ShouldGoToOrderDesk::is(false), HasPendingOrder::is(false)]);

    let (planner, state) = create_planner!({
        actions: [
            (TakeOrder, take_order),
            (ServeOrder, serve),
            (ProduceLemonade, produce),
            (GoToLemonadeMaker, go_to_maker),
            (GoToChair, go_to_chair),
            (GoToOrderDesk, go_to_desk),
            (Rest, rest),
        ],
        state: [
            CarryingItem(Item::Nothing),
            HasPendingOrder(false),
            AtOrderDesk(false),
            AtLemonadeMaker(false),
            AtChair(false),
            ShouldGoToOrderDesk(false),
            ServedOrder(false),
            OrderTaken(false),
            Energy(0.7),
        ],
        goals: [worker_goal],
    });

    commands
        .spawn((
            Agent,
            Name::new("Worker"),
            Worker,
            InheritedVisibility::default(),
            planner,
            state,
            Transform::from_translation(pos),
        ))
        .with_children(|sub| {
            sub.spawn((
                Transform::from_translation(Vec3::new(50.0, 0.0, 10.0)),
                Text2d("".into()),
                TextFont { font_size: 12.0, ..default() },
                bevy::sprite::Anchor::TOP_LEFT,
                StateDebugText,
            ));
        });
}

// Derived data

fn derive_at_stations(
    q_desk: Query<&Transform, With<OrderDesk>>,
    q_maker: Query<&Transform, With<LemonadeMaker>>,
    q_chair: Query<&Transform, With<Chair>>,
    mut sets: ParamSet<(
        Query<(&Transform, &mut AtOrderDesk, &mut AtLemonadeMaker), With<Customer>>,
        Query<(&Transform, &mut AtOrderDesk, &mut AtLemonadeMaker, &mut AtChair), With<Worker>>,
    )>,
) {
    let desk_t = q_desk
        .single()
        .expect("Exactly one OrderDesk expected");
    let maker_t = q_maker
        .single()
        .expect("Exactly one LemonadeMaker expected");

    {
        let mut q_customers = sets.p0();
        for (t, mut at_desk, mut at_maker) in q_customers.iter_mut() {
            let target = desk_pos_for_customer(desk_t);
            at_desk.0 = t.translation.distance(target) <= ARRIVAL_RADIUS;
            at_maker.0 = t.translation.distance(maker_t.translation) <= ARRIVAL_RADIUS;
        }
    }
    let chair_t = q_chair
        .single()
        .expect("Exactly one Chair expected");

    {
        let mut q_workers = sets.p1();
        for (t, mut at_desk, mut at_maker, mut at_chair) in q_workers.iter_mut() {
            let target = desk_pos_for_worker(desk_t);
            at_desk.0 = t.translation.distance(target) <= ARRIVAL_RADIUS;
            at_maker.0 = t.translation.distance(maker_t.translation) <= ARRIVAL_RADIUS;
            at_chair.0 = t.translation.distance(chair_t.translation) <= ARRIVAL_RADIUS;
        }
    }
}

fn derive_has_pending_order(
    mut q_workers: Query<&mut HasPendingOrder, With<Worker>>,
    q_desk: Query<&OrderDesk>,
) {
    let desk = q_desk
        .single()
        .expect("Exactly one OrderDesk expected");
    let pending = desk.current_order.is_some();
    for mut has in q_workers.iter_mut() {
        has.0 = pending;
    }
}

fn derive_can_take_order(
    mut q_desk: Query<(
        &Transform,
        &mut OrderDesk,
        Option<&OrderSession>,
        Option<&ServeSession>,
    )>,
    q_customers: Query<&AtOrderDesk, With<Customer>>,
    q_workers: Query<&AtOrderDesk, With<Worker>>,
) {
    let (_t, mut desk, order_session, serve_session) = q_desk
        .single_mut()
        .expect("Exactly one OrderDesk expected");
    let cust_here = q_customers.iter().any(|a| a.0);
    let worker_here = q_workers.iter().any(|a| a.0);
    let order_active = order_session.and_then(|s| s.timer.as_ref()).is_some();
    let serve_active = serve_session.and_then(|s| s.timer.as_ref()).is_some();
    desk.can_take_order = cust_here
        && worker_here
        && desk.current_order.is_none()
        && !order_active
        && !serve_active;
}

// General continuous behaviours

fn update_thirst(time: Res<Time>, mut q: Query<&mut Thirst, With<Customer>>) {
    for mut thirst in q.iter_mut() {
        thirst.0 += time.delta_secs_f64() * THIRST_RATE;
        if thirst.0 > 100.0 {
            thirst.0 = 100.0;
        }
    }
}

fn update_energy(
    time: Res<Time>,
    mut q: Query<(&AtChair, &mut Energy, Option<&ProduceLemonade>), With<Worker>>,
) {
    let dt = time.delta_secs_f64();
    for (at_chair, mut energy, producing) in q.iter_mut() {
        let delta = if at_chair.0 {
            ENERGY_GAIN_RATE * dt
        } else if producing.is_some() {
            // Slightly faster drain while producing
            -(ENERGY_DECAY_RATE * ENERGY_DECAY_PRODUCE_MULT) * dt
        } else {
            -ENERGY_DECAY_RATE * dt
        };
        energy.0 = (energy.0 + delta).clamp(0.0, 1.0);
    }
}

fn update_customer_goal(mut commands: Commands, mut q: Query<(Entity, &mut Planner, &Thirst), With<Customer>>) {
    for (e, mut planner, thirst) in q.iter_mut() {
        if thirst.0 > THIRST_THRESHOLD {
            if planner.goals.is_empty() {
                let goal = Goal::from_reqs(&[Thirst::is_less(THIRST_THRESHOLD)]);
                set_goals_and_replan(&mut commands, e, &mut planner, vec![goal]);
            }
        } else {
            if !planner.goals.is_empty() {
                planner.goals.clear();
                planner.current_plan = None;
                planner.current_action = None;
            }
        }
    }
}

fn update_worker_goal(
    mut commands: Commands,
    mut q: Query<(
        Entity,
        &mut Planner,
        &ShouldGoToOrderDesk,
        &HasPendingOrder,
        &Energy,
    ), With<Worker>>,
) {
    for (e, mut planner, should_go, pending, energy) in q.iter_mut() {
        // Rest goal has priority when energy is low
        if energy.0 < ENERGY_LOW_THRESH {
            let rest_goal = Goal::from_reqs(&[Energy::is_more(ENERGY_TARGET)]);
            set_goals_and_replan(&mut commands, e, &mut planner, vec![rest_goal]);
            continue;
        }

        // Otherwise, handle work/demand goal
        let needs_work = should_go.0 || pending.0;
        if needs_work {
            let goal = Goal::from_reqs(&[ShouldGoToOrderDesk::is(false), HasPendingOrder::is(false)]);
            set_goals_and_replan(&mut commands, e, &mut planner, vec![goal]);
        } else if !planner.goals.is_empty() {
            planner.goals.clear();
            planner.current_plan = None;
            planner.current_action = None;
        }
    }
}

fn trigger_replanning(mut commands: Commands, q: Query<(Entity, &Planner)>) {
    for (e, planner) in q.iter() {
        if !planner.goals.is_empty() && planner.current_plan.is_none() {
            commands.entity(e).trigger(UpdatePlan::from);
        }
    }
}

// Execution systems (movement and actions)

fn handle_move_to(mut commands: Commands, time: Res<Time>, mut q: Query<(Entity, &MoveTo, &mut Transform)>) {
    for (e, move_to, mut t) in q.iter_mut() {
        let dest = move_to.0;
        if t.translation.distance(dest) > ARRIVAL_RADIUS {
            let dir = (dest - t.translation).normalize();
            t.translation += dir * MOVE_SPEED * time.delta_secs();
        } else {
            commands.entity(e).remove::<MoveTo>();
        }
    }
}

fn handle_go_to_order_desk(
    mut commands: Commands,
    q_desk: Query<&Transform, With<OrderDesk>>,
    mut sets: ParamSet<(
        Query<(Entity, &Transform, &GoToOrderDesk, &mut AtOrderDesk), (With<Customer>, Without<MoveTo>)>,
        Query<(
            Entity,
            &Transform,
            &GoToOrderDesk,
            &mut AtOrderDesk,
            &mut OrderTaken,
        ), (With<Worker>, Without<MoveTo>)>,
    )>,
) {
    let desk_t = q_desk
        .single()
        .expect("Exactly one OrderDesk expected");

    {
        let mut q_cust = sets.p0();
        for (e, t, _a, mut at) in q_cust.iter_mut() {
            let target = desk_pos_for_customer(desk_t);
            if move_or_arrive(&mut commands, e, t, target) {
                at.0 = true;
                commands.entity(e).remove::<GoToOrderDesk>();
            }
        }
    }
    {
        let mut q_work = sets.p1();
        for (e, t, _a, mut at, mut taken) in q_work.iter_mut() {
            let target = desk_pos_for_worker(desk_t);
            if move_or_arrive(&mut commands, e, t, target) {
                at.0 = true;
                taken.0 = false;
                commands.entity(e).remove::<GoToOrderDesk>();
            }
        }
    }
}

fn handle_take_order(
    mut commands: Commands,
    q_place_cust: Query<&AtOrderDesk, (With<Customer>, With<PlaceOrder>)>,
    mut q: Query<(Entity, &TakeOrder, &AtOrderDesk), With<Worker>>,
) {
    let any_customer_ordering_here = q_place_cust.iter().any(|a| a.0);
    for (e, _a, at_desk) in q.iter_mut() {
        remove_if::<TakeOrder>(&mut commands, e, !at_desk.0 || !any_customer_ordering_here);
    }
}

fn handle_go_to_lemonade_maker(
    mut commands: Commands,
    q_maker: Query<&Transform, With<LemonadeMaker>>,
    mut q: Query<(
        Entity,
        &Transform,
        &GoToLemonadeMaker,
        &mut AtLemonadeMaker,
        &mut AtOrderDesk,
        &OrderTaken,
    ), Without<MoveTo>>,
) {
    let maker_t = q_maker
        .single()
        .expect("Exactly one LemonadeMaker expected");
    for (e, t, _a, mut at_maker, mut at_desk, taken) in q.iter_mut() {
        if !taken.0 {
            commands.entity(e).remove::<GoToLemonadeMaker>();
            continue;
        }
        if move_or_arrive(&mut commands, e, t, maker_t.translation) {
            at_maker.0 = true;
            at_desk.0 = false;
            commands.entity(e).remove::<GoToLemonadeMaker>();
        }
    }
}

fn handle_go_to_chair(
    mut commands: Commands,
    q_chair: Query<&Transform, With<Chair>>,
    mut q: Query<(Entity, &Transform, &GoToChair, &mut AtChair, &mut AtOrderDesk, &mut AtLemonadeMaker), Without<MoveTo>>,
) {
    let chair_t = q_chair
        .single()
        .expect("Exactly one Chair expected");
    for (e, t, _a, mut at_chair, mut at_desk, mut at_maker) in q.iter_mut() {
        if move_or_arrive(&mut commands, e, t, chair_t.translation) {
            at_chair.0 = true;
            at_desk.0 = false;
            at_maker.0 = false;
            commands.entity(e).remove::<GoToChair>();
        }
    }
}

fn handle_wait_for_order(
    mut commands: Commands,
    q_orders: Query<&Order>,
    mut q: Query<(Entity, &WaitForOrder, &Customer, &OrderReady)>,
) {
    for (e, _a, cust, ready) in q.iter_mut() {
        if let Some(o) = cust.order {
            if q_orders.get(o).is_err() || ready.0 {
                commands.entity(e).remove::<WaitForOrder>();
            }
        } else {
            commands.entity(e).remove::<WaitForOrder>();
        }
    }
}

fn handle_produce_lemonade(
    mut commands: Commands,
    time: Res<Time>,
    mut q: Query<(
        Entity,
        &ProduceLemonade,
        &mut CarryingItem,
        &AtLemonadeMaker,
        &OrderTaken,
        Option<&mut ActionProgress>,
    )>,
    q_desk: Query<&OrderDesk>,
    mut q_orders: Query<&mut Order>,
) {
    let desk = q_desk
        .single()
        .expect("Exactly one OrderDesk expected");
    for (e, _a, mut carry, at_maker, taken, progress) in q.iter_mut() {
        if !at_maker.0 {
            continue;
        }
        if !taken.0 {
            // Plan requires TakeOrder, but if invalidated, drop action to replan
            commands.entity(e).remove::<ProduceLemonade>();
            continue;
        }
        let Some(order_e) = desk.current_order else {
            commands.entity(e).remove::<ProduceLemonade>();
            continue;
        };
        if let Ok(mut order) = q_orders.get_mut(order_e) {
            if order.items_to_produce.is_empty() {
                commands.entity(e).remove::<ProduceLemonade>();
                continue;
            }
            if progress_or_start(&mut commands, e, progress, PRODUCE_TIME, &time) {
                let _ = order.items_to_produce.pop_front();
                carry.0 = Item::Lemonade;
                commands.entity(e).remove::<ProduceLemonade>();
                commands.entity(e).remove::<ActionProgress>();
            }
        } else {
            commands.entity(e).remove::<ProduceLemonade>();
        }
    }
}

fn handle_serve_order(mut commands: Commands, mut q: Query<(Entity, &ServeOrder, &AtOrderDesk, &CarryingItem), With<Worker>>) {
    for (e, _a, at_desk, carry) in q.iter_mut() {
        remove_if::<ServeOrder>(&mut commands, e, !at_desk.0 || carry.0 != Item::Lemonade);
    }
}

fn handle_pickup_order(mut commands: Commands, mut q: Query<(Entity, &PickupOrder, &AtOrderDesk), With<Customer>>) {
    for (e, _a, at_desk) in q.iter_mut() {
        remove_if::<PickupOrder>(&mut commands, e, !at_desk.0);
    }
}

fn handle_drink_lemonade(
    mut commands: Commands,
    time: Res<Time>,
    mut q: Query<(
        Entity,
        &DrinkLemonade,
        &mut CarryingItem,
        &mut Thirst,
        Option<&mut ActionProgress>,
    )>,
) {
    for (e, _a, mut carrying, mut thirst, progress) in q.iter_mut() {
        if progress.is_none() {
            commands.entity(e).insert(MoveTo(random_wander_target()));
        }
        if progress_or_start(&mut commands, e, progress, DRINK_TIME, &time) {
            carrying.0 = Item::Nothing;
            thirst.0 = (thirst.0 - 10.0).max(0.0);
            commands.entity(e).remove::<DrinkLemonade>();
            commands.entity(e).remove::<ActionProgress>();
        }
    }
}

fn handle_rest(mut commands: Commands, mut q: Query<(Entity, &Rest, &AtChair, &Energy)>) {
    for (e, _a, at_chair, energy) in q.iter_mut() {
        // Rest only meaningful at chair; drop to replan
        if !at_chair.0 {
            remove_if::<Rest>(&mut commands, e, true);
            continue;
        }
        remove_if::<Rest>(&mut commands, e, energy.0 >= ENERGY_TARGET);
    }
}

fn call_worker_to_desk(
    q_desk: Query<&OrderDesk>,
    q_place_cust: Query<&AtOrderDesk, (With<Customer>, With<PlaceOrder>)>,
    mut q_worker: Query<(&mut ShouldGoToOrderDesk, &OrderTaken), With<Worker>>,
) {
    let desk = q_desk
        .single()
        .expect("Exactly one OrderDesk expected");
    let customer_ordering_here = q_place_cust.iter().any(|a| a.0);
    for (mut should, taken) in q_worker.iter_mut() {
        if customer_ordering_here && desk.current_order.is_none() && !taken.0 {
            should.0 = true;
        }
    }
}

fn handle_customer_wander(
    mut commands: Commands,
    time: Res<Time>,
    q_actions: Query<(Entity, &dyn ActionComponent)>,
    mut q_customers: Query<(Entity, Option<&mut IdleWanderTimer>), With<Customer>>,
) {
    for (e, timer_opt) in q_customers.iter_mut() {
        if has_any_action(e, &q_actions) {
            continue;
        }

        match timer_opt {
            Some(mut t) => {
                if t.0.tick(time.delta()).just_finished() {
                    commands.entity(e).insert(MoveTo(random_wander_target()));
                    let mut rng = rand::rng();
                    let next = rng.random_range(6.0..15.0);
                    t.0 = Timer::from_seconds(next, TimerMode::Once);
                }
            }
            None => {
                let mut rng = rand::rng();
                let next = rng.random_range(6.0..15.0);
                commands
                    .entity(e)
                    .insert(IdleWanderTimer(Timer::from_seconds(next, TimerMode::Once)));
            }
        }
    }
}

// Calling this "UI" might be a stretch

fn draw_state_debug(
    q_planners: Query<(Entity, &Name, &Children), With<Planner>>,
    q_actions: Query<(Entity, &dyn ActionComponent)>,
    q_datums: Query<(Entity, &dyn DatumComponent)>,
    q_child: Query<Entity, With<StateDebugText>>,
    mut text_writer: Text2dWriter,
) {
    for (entity, name, children) in q_planners.iter() {
        let current_action = {
            let mut curr = "Idle";
            for (_e, actions) in q_actions.get(entity).iter() {
                if let Some(action) = actions.iter().next() {
                    curr = action.action_type_name();
                }
            }
            curr
        };
        let state = build_state_string(entity, &q_datums);
        for child in children.iter() {
            let text = q_child.get(child).unwrap();
            *text_writer.text(text, 0) = format!("{name}\n{current_action}\nEntity: {entity}\n---\n{state}");
        }
    }
}

fn build_state_string(entity: Entity, q_datums: &Query<(Entity, &dyn DatumComponent)>) -> String {
    let mut state = String::new();
    for (_e, data) in q_datums.get(entity).iter() {
        for datum in data.iter() {
            let v = match datum.field_value() {
                Datum::Bool(v) => v.to_string(),
                Datum::F64(v) => format!("{v:.2}"),
                Datum::I64(v) => format!("{v}"),
                Datum::Enum(v) => format!("{v}"),
            };
            state = format!("{state}\n{}: {}", datum.field_key(), v);
        }
    }
    state
}

fn draw_ui(
    mut gizmos: Gizmos,
    q_customer: Query<&Transform, With<Customer>>,
    q_workers: Query<&Transform, With<Worker>>,
    q_lemonade_makers: Query<&Transform, With<LemonadeMaker>>,
    q_order_desks: Query<&Transform, With<OrderDesk>>,
    q_chair: Query<&Transform, With<Chair>>,
) {
    gizmos
        .grid_2d(Vec2::ZERO, UVec2::new(16, 9), Vec2::new(80., 80.), Srgba::new(0.1, 0.1, 0.1, 0.5))
        .outer_edges();

    for t in q_customer.iter() {
        gizmos.circle_2d(t.translation.xy(), 16., GREEN);
    }
    for t in q_workers.iter() {
        gizmos.circle_2d(t.translation.xy(), 16., BLUE);
    }
    for t in q_lemonade_makers.iter() {
        gizmos.rect_2d(t.translation.xy(), Vec2::new(20.0, 20.0), GOLD);
    }
    for t in q_order_desks.iter() {
        gizmos.rect_2d(t.translation.xy(), Vec2::new(40.0, 40.0), BLUE_VIOLET);
    }
    for t in q_chair.iter() {
        gizmos.rect_2d(t.translation.xy(), Vec2::new(20.0, 20.0), Srgba::new(0.4, 0.2, 0.1, 1.0));
    }
}

fn update_money_text(q: Query<Entity, With<MoneyText>>, money: Res<Money>, mut text: Text2dWriter) {
    for e in q.iter() {
        *text.text(e, 0) = format!("Money: {}", money.0);
    }
}

// Runtime validation

fn check_invariants(
    q_desk: Query<(&OrderDesk, Option<&OrderSession>)>,
    q_cust_at: Query<&AtOrderDesk, With<Customer>>,
    q_work_at: Query<&AtOrderDesk, With<Worker>>,
    q_workers_pending: Query<&HasPendingOrder, With<Worker>>,
    q_carry: Query<&CarryingItem, With<Customer>>,
    q_ready: Query<&OrderReady, With<Customer>>,
    q_placed: Query<&PlacedOrder, With<Customer>>,
    q_planners: Query<(Entity, &Planner)>,
    q_actions: Query<(Entity, &dyn ActionComponent)>,
    q_progress: Query<Entity, With<ActionProgress>>,
    q_desk_t: Query<&Transform, With<OrderDesk>>,
    q_cust_t: Query<(&Transform, &AtOrderDesk), With<Customer>>,
    q_work_t: Query<(&Transform, &AtOrderDesk), With<Worker>>,
) {
    // Desk readiness equality
    let (desk, session) = match q_desk.single() {
        Ok(d) => d,
        Err(_) => return,
    };
    let cust_here = q_cust_at.iter().any(|a| a.0);
    let worker_here = q_work_at.iter().any(|a| a.0);
    let session_active = session.and_then(|s| s.timer.as_ref()).is_some();
    let expect_can = cust_here && worker_here && desk.current_order.is_none() && !session_active;
    if desk.can_take_order != expect_can {
        error!(
            "Invariant: can_take_order mismatch: desk.can_take_order={}, expected={} (cust_here={}, worker_here={}, has_order={})",
            desk.can_take_order,
            expect_can,
            cust_here,
            worker_here,
            desk.current_order.is_some()
        );
    }

    // HasPendingOrder consistency for workers
    let pending = desk.current_order.is_some();
    for has in q_workers_pending.iter() {
        if has.0 != pending {
            error!(
                "Invariant: HasPendingOrder mismatch on worker: has={}, expected={} (has_order={})",
                has.0,
                pending,
                pending
            );
        }
    }

    // If a customer is 'ready', they shouldn't be carrying an item yet
    for (ready, carry) in q_ready.iter().zip(q_carry.iter()) {
        if ready.0 && carry.0 != Item::Nothing {
            error!(
                "Invariant: Customer OrderReady but carrying {:?} (expected Nothing)",
                carry.0
            );
        }
    }

    // If a customer has placed an order, either an order exists or it's already ready
    let mut any_violation = false;
    for (placed, ready) in q_placed.iter().zip(q_ready.iter()) {
        if placed.0 && !pending && !ready.0 {
            any_violation = true;
        }
    }
    if any_violation {
        error!(
            "Invariant: PlacedOrder=true but neither desk has order nor customer OrderReady=true"
        );
    }

    // Single active action per planner, and ActionProgress implies an action
    for (entity, _planner) in q_planners.iter() {
        let count = q_actions
            .get(entity)
            .map(|(_, a)| a.iter().count())
            .unwrap_or(0);
        if count > 1 {
            error!(
                "Invariant: Multiple actions ({count}) active on planner entity {entity:?}"
            );
        }
        if q_progress.get(entity).is_ok() && count == 0 {
            error!(
                "Invariant: ActionProgress present but no active action on entity {entity:?}"
            );
        }
    }

    // Positional derivations consistent
    if let (Ok(desk_t), Ok(_)) = (q_desk_t.single(), q_desk.single()) {
        for (t, at) in q_cust_t.iter() {
            let target = desk_t.translation + DESK_CUSTOMER_OFFSET;
            let near = t.translation.distance(target) <= ARRIVAL_RADIUS + 0.001;
            if at.0 && !near {
                error!(
                    "Invariant: Customer AtOrderDesk=true but position not near desk offset"
                );
            }
        }
        for (t, at) in q_work_t.iter() {
            let target = desk_pos_for_worker(desk_t);
            let near = t.translation.distance(target) <= ARRIVAL_RADIUS + 0.001;
            if at.0 && !near {
                error!(
                    "Invariant: Worker AtOrderDesk=true but position not near desk offset"
                );
            }
        }
    }
}

fn handle_order_session(
    mut commands: Commands,
    time: Res<Time>,
    mut q_desk: Query<(&mut OrderDesk, &mut OrderSession)>,
    mut q_cust: Query<(Entity, &mut PlacedOrder, &mut OrderReady, &mut Customer, &AtOrderDesk), With<PlaceOrder>>,
    mut q_work: Query<(Entity, &mut OrderTaken, &mut ShouldGoToOrderDesk, &AtOrderDesk), With<TakeOrder>>,
) {
    let Ok((mut desk, mut session)) = q_desk.single_mut() else { return; };

    let had_timer = session.timer.is_some();
    let cust_present = session
        .customer
        .and_then(|ce| q_cust.get_mut(ce).ok())
        .map(|(_, _, _, _, at)| at.0)
        .unwrap_or(false);
    let work_present = session
        .worker
        .and_then(|we| q_work.get_mut(we).ok())
        .map(|(_, _, _, at)| at.0)
        .unwrap_or(false);

    let start_secs = if session.timer.is_none() && desk.current_order.is_none() {
        let customer = q_cust
            .iter_mut()
            .find_map(|(e, _, _, _, at)| if at.0 { Some(e) } else { None });
        let worker = q_work
            .iter_mut()
            .find_map(|(e, _, _, at)| if at.0 { Some(e) } else { None });
        if let (Some(ce), Some(we)) = (customer, worker) {
            session.customer = Some(ce);
            session.worker = Some(we);
            Some(ORDERING_TIME)
        } else {
            None
        }
    } else {
        None
    };

    let finished = session_step(&time, &mut session.timer, cust_present && work_present, start_secs);

    if had_timer && !cust_present || had_timer && !work_present {
        session.customer = None;
        session.worker = None;
    }

    if finished {
        if let (Some(ce), Some(we)) = (session.customer.take(), session.worker.take()) {
            if let Ok((_, mut placed, mut ready, mut cust, _)) = q_cust.get_mut(ce) {
                let order = Order { items_to_produce: VecDeque::from([Item::Lemonade]) };
                let e_order = commands.spawn((Name::new("Order"), order)).id();
                desk.current_order = Some(e_order);
                placed.0 = true;
                ready.0 = false;
                cust.order = Some(e_order);
                commands.entity(ce).remove::<PlaceOrder>();
            }
            if let Ok((_, mut taken, mut should, _)) = q_work.get_mut(we) {
                taken.0 = true;
                should.0 = false;
                commands.entity(we).remove::<TakeOrder>();
            }
        }
    }
}

fn handle_serve_session(
    mut commands: Commands,
    time: Res<Time>,
    mut q_desk: Query<(&mut OrderDesk, &mut ServeSession)>,
    mut sets: ParamSet<(
        Query<
            (
                Entity,
                &mut CarryingItem,
                &AtOrderDesk,
                &mut OrderTaken,
                &mut ServedOrder,
            ),
            (With<ServeOrder>, With<Worker>),
        >,
        Query<
            (
                Entity,
                &mut CarryingItem,
                &mut PlacedOrder,
                &mut OrderReady,
                &AtOrderDesk,
            ),
            (With<PickupOrder>, With<Customer>),
        >,
        Query<
            (
                Entity,
                &mut PlacedOrder,
                &mut OrderReady,
                &AtOrderDesk,
                &CarryingItem,
            ),
            (With<WaitForOrder>, With<Customer>),
        >,
    )>,
    mut q_orders: Query<(Entity, &mut Order)>,
    mut money: ResMut<Money>,
) {
    let Ok((mut desk, mut session)) = q_desk.single_mut() else { return; };

    let had_timer = session.timer.is_some();
    let cust_present = {
        let mut q_cust = sets.p1();
        session
            .customer
            .and_then(|ce| q_cust.get_mut(ce).ok())
            .map(|(_, _, _, _, at)| at.0)
            .unwrap_or(false)
    };
    let work_present = {
        let mut q_work = sets.p0();
        session
            .worker
            .and_then(|we| q_work.get_mut(we).ok())
            .map(|(_, _, at, _, _)| at.0)
            .unwrap_or(false)
    };

    let start_secs = if session.timer.is_none() && desk.current_order.is_some() {
        let work = {
            let mut q_work = sets.p0();
            q_work
                .iter_mut()
                .find_map(|(e, wcarry, at, _, _)| if at.0 && wcarry.0 == Item::Lemonade { Some(e) } else { None })
        };
        if let Some(we) = work {
            let cust_pickup = {
                let mut q_cust = sets.p1();
                q_cust
                    .iter_mut()
                    .find_map(|(e, ccarry, _, _, at)| if at.0 && ccarry.0 == Item::Nothing { Some(e) } else { None })
            };
            let ce = if let Some(ce) = cust_pickup {
                Some(ce)
            } else {
                let mut q_wait = sets.p2();
                if let Some((e, _placed, _ready, _at, _carry)) = q_wait
                    .iter_mut()
                    .find(|(_, _, _, at, carry)| at.0 && carry.0 == Item::Nothing)
                {
                    commands.entity(e).remove::<WaitForOrder>();
                    commands.entity(e).insert(PickupOrder);
                    Some(e)
                } else {
                    None
                }
            };
            if let Some(ce) = ce {
                session.customer = Some(ce);
                session.worker = Some(we);
                Some(SERVE_TIME)
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    let finished = session_step(&time, &mut session.timer, cust_present && work_present, start_secs);

    if had_timer && (!cust_present || !work_present) {
        session.customer = None;
        session.worker = None;
    }

    if finished {
        if let (Some(ce), Some(we)) = (session.customer.take(), session.worker.take()) {
            if let Some(order_e) = desk.current_order.take() {
                if q_orders.get_mut(order_e).is_ok() {
                    let mut ok = false;
                    {
                        let mut q_work = sets.p0();
                        if let Ok((_, mut wcarry, _, mut taken, mut served)) = q_work.get_mut(we) {
                            wcarry.0 = Item::Nothing;
                            served.0 = true;
                            taken.0 = false;
                            ok = true;
                        }
                    }
                    if ok {
                        let mut q_cust = sets.p1();
                        if let Ok((_, mut ccarry, mut placed, mut ready, _)) = q_cust.get_mut(ce) {
                            ccarry.0 = Item::Lemonade;
                            placed.0 = false;
                            ready.0 = false;
                            money.0 += 3;
                            commands.entity(we).remove::<ServeOrder>();
                            commands.entity(ce).remove::<PickupOrder>();
                            commands.entity(order_e).despawn();
                            commands.entity(ce).insert(MoveTo(random_wander_target()));
                        }
                    }
                }
            }
        }
    }
}
