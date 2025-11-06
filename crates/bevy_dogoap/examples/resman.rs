//! A little restaurant manager
//!
//! Customer > Has Thirst that they want to fullfil
//! Worker > Wants to fulfill orders to increase profits of business
//!
//! High-level, we have the following:
//!
//! Agent - Shared behaviour between Customer and Worker
//! Customer - Has Thirst, wants to satisfy it somehow
//! Worker - Wants to increase income of business
//!
//! Customer has Actions:
//! - `GoToOrderDesk`, `MakeOrder`, `ConsumeOrder`, `ConsumeInventory`
//!
//! Worker has Actions:
//! - `GoToOrderDesk`, `TakeOrder`, `MakeProduct`, `MoveProduct`, `HandOverOrder`
//!
//! Sequence Diagram for the full flow of actions (paste into <https://sequencediagram.org/)>:
//!
//! Customer->Order Desk: `GoToOrderDesk`
//! Order Desk->Worker: `RequestWorker`
//! Worker->Order Desk: `GoToOrderDesk`
//! Customer->Order Desk: `PlaceOrder`
//! Worker->Order Desk: `TakeOrder`
//! Customer->Order Desk: `WaitForOrder`
//! Worker->Lemonade Maker: `GoToLemonadeMaker`
//! Lemonade Maker->Worker: `MakeLemonade`
//! Worker->Order Desk: `FinishOrder`
//! Customer->Order Desk: `PickupLemonade`
//! Customer->Customer: `DrinkLemonade`

use std::{
    collections::{HashMap, VecDeque},
    time::Duration,
};

use bevy::{
    color::palettes::css::*,
    prelude::{Camera2d, *},
    time::common_conditions::on_timer,
    window::{Window, WindowPlugin},
};
use bevy_dogoap::prelude::*;

fn main() {
    let mut app = App::new();
    // Customer components + actions
    register_components!(
        app,
        [
            Thirst,
            CarryingItem,
            PlacedOrder,
            OrderReady,
            AtOrderDesk,
            ShouldGoToOrderDesk
        ]
    );
    register_actions!(
        app,
        [
            DrinkLemonade,
            PickupLemonade,
            WaitForOrder,
            PlaceOrder,
            GoToOrderDesk
        ]
    );
    // Worker components + actions
    register_components!(
        app,
        [Energy, AtLemonadeMaker, ServedOrder, ShouldGoToOrderDesk]
    );
    register_actions!(app, [Rest, ServeOrder, ProduceLemonade, GoToLemonadeMaker]);

    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            canvas: Some("#example-canvas".into()),
            ..default()
        }),
        ..default()
    }))
    .add_plugins(DogoapPlugin::default())
    .add_systems(Startup, setup)
    .add_systems(Update, (draw_state_debug, draw_ui))
    // Systems that always affects needs
    .add_systems(FixedUpdate, update_thirst)
    // Systems that handle actions
    .add_systems(
        FixedUpdate,
        (
            update_plan.run_if(on_timer(Duration::from_millis(500))),
            handle_pickup_lemonade,
            handle_drink_lemonade,
            handle_place_order,
            handle_wait_for_order,
            handle_go_to_order_desk,
            handle_move_to,
            handle_call_worker_to_empty_order_desk,
            handle_idle.run_if(on_timer(Duration::from_millis(800))),
        ),
    )
    .run();
}

// LocalFields for customer

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
struct ShouldGoToOrderDesk(bool);

// Actions for customer

#[derive(Component, Clone, Default, ActionComponent)]
struct DrinkLemonade;

#[derive(Component, Clone, Default, ActionComponent)]
struct PickupLemonade;

#[derive(Component, Clone, Default, ActionComponent)]
struct WaitForOrder;

#[derive(Component, Clone, Default, ActionComponent)]
struct PlaceOrder;

#[derive(Component, Clone, Default, ActionComponent)]
struct GoToOrderDesk;

// DatumComponents for worker

#[derive(Component, Clone, DatumComponent)]
struct Energy(f64);

#[derive(Component, Clone, DatumComponent)]
struct ServedOrder(bool);

#[derive(Component, Clone, DatumComponent)]
struct AtLemonadeMaker(bool);

#[derive(Component, Clone, DatumComponent)]
struct Idling(bool);

// Actions for worker

#[derive(Component, Clone, Default, ActionComponent)]
struct Rest;

#[derive(Component, Clone, Default, ActionComponent)]
struct ServeOrder;

#[derive(Component, Clone, Default, ActionComponent)]
struct ProduceLemonade;

#[derive(Component, Clone, Default, ActionComponent)]
struct GoToLemonadeMaker;

#[derive(Component, Clone, Default, ActionComponent)]
struct Idle;

// Markers

#[derive(Component)]
struct Agent;

#[derive(Component, Default)]
struct Customer {
    order: Option<Entity>,
}

#[derive(Component)]
struct Worker;

#[derive(Clone, Default, Copy, Reflect)]
enum Item {
    #[default]
    Nothing,
    // Actual items:
    Lemonade,
}

#[derive(Component)]
struct LemonadeMaker;

#[derive(Component)]
struct Order {
    items_to_produce: VecDeque<Item>,
    _items: Vec<Item>,
    _owner: Entity,
}

#[derive(Component, Default)]
struct OrderDesk {
    assigned_customer: Option<Entity>,
    assigned_worker: Option<Entity>,
    can_take_order: bool, // set to true when both customer and worker present
    current_order: Option<Entity>,
}

#[derive(Component)]
struct MoveTo(Vec3);

#[derive(Component)]
struct StateDebugText;

fn setup(mut commands: Commands) {
    // Spawn customers
    for _i in 0..1 {
        let not_thirsty_goal = Goal::from_reqs(&[Thirst::is_less(1.0)]);

        // Requires us to carry a lemonade, results in us having 10 less thirst + carrying Nothing
        let drink_lemonade_action = DrinkLemonade::action()
            .with_precondition(CarryingItem::is(Item::Lemonade))
            .with_mutator(CarryingItem::set(Item::Nothing))
            .with_mutator(Thirst::decrease(10.0));

        // Requires us to not be carrying nothing, and leads to us having a lemonade
        let pickup_lemonade_action = PickupLemonade::action()
            .with_precondition(CarryingItem::is(Item::Nothing))
            .with_precondition(OrderReady::is(true))
            .with_precondition(AtOrderDesk::is(true))
            .with_mutator(CarryingItem::set(Item::Lemonade));

        // Requires us to having placed an order, order not yet ready and we're at the order desk
        let wait_for_order_action = WaitForOrder::action()
            .with_precondition(PlacedOrder::is(true))
            .with_precondition(OrderReady::is(false))
            .with_precondition(AtOrderDesk::is(true))
            .with_mutator(OrderReady::set(true));

        // Requires us to not having placed an order previously, and we're at the ordering desk
        let place_order_action = PlaceOrder::action()
            .with_precondition(PlacedOrder::is(false))
            .with_precondition(AtOrderDesk::is(true))
            .with_mutator(PlacedOrder::set(true));

        let go_to_order_desk_action = GoToOrderDesk::action()
            .with_precondition(AtOrderDesk::is(false))
            .with_mutator(AtOrderDesk::set(true));

        let (planner, components) = create_planner!({
            actions: [
                (DrinkLemonade, drink_lemonade_action),
                (PickupLemonade, pickup_lemonade_action),
                (WaitForOrder, wait_for_order_action),
                (PlaceOrder, place_order_action),
                (GoToOrderDesk, go_to_order_desk_action),
            ],
            state: [
                Thirst(0.0),
                CarryingItem(Item::Nothing),
                PlacedOrder(false),
                OrderReady(false),
                AtOrderDesk(false),
            ],
            goals: [not_thirsty_goal],
        });

        commands
            .spawn((
                Agent,
                Name::new("Customer"),
                Customer::default(),
                Visibility::default(),
                planner,
                components,
                Transform::from_xyz(-200.0, -100.0, 1.0),
            ))
            .with_children(|subcommands| {
                subcommands.spawn((
                    Transform::from_translation(Vec3::new(-70.0, 0.0, 10.0)),
                    Text2d("".into()),
                    TextFont {
                        font_size: 12.0,
                        ..default()
                    },
                    bevy::sprite::Anchor::TOP_LEFT,
                    StateDebugText,
                ));
            });
    }

    // Spawn worker
    for _i in 0..1 {
        // Now for the worker

        // Final outcome for the worker is increasing the amount of money, always
        // We trick the agent into performing our actions forever by having a:
        // ServedOrder DatumComponent that the agent wants to set to true,
        // but at runtime it can never actually get there.

        // In order to set ServedOrder to true, the agent needs to run ServeOrder

        let at_order_desk_goal = Goal::from_reqs(&[AtOrderDesk::is(true)]);
        let idle_goal = Goal::from_reqs(&[Idling::is(true)]);

        let serve_order_action = ServeOrder::action()
            .with_precondition(CarryingItem::is(Item::Lemonade))
            .with_precondition(AtOrderDesk::is(true))
            .with_mutator(ServedOrder::set(true));

        let produce_lemonade_action = ProduceLemonade::action()
            .with_precondition(CarryingItem::is(Item::Nothing))
            .with_precondition(AtLemonadeMaker::is(true))
            .with_mutator(CarryingItem::set(Item::Lemonade));

        let go_to_lemonade_maker_action = GoToLemonadeMaker::action()
            .with_precondition(AtLemonadeMaker::is(false))
            .with_mutator(AtLemonadeMaker::set(true));

        let rest_action = Rest::action()
            .with_precondition(Energy::is_less(10.0))
            .with_mutator(Energy::increase(50.0));

        let go_to_order_desk_action = GoToOrderDesk::action()
            .with_precondition(AtOrderDesk::is(false))
            .with_precondition(ShouldGoToOrderDesk::is(true))
            .with_mutator(AtOrderDesk::set(true));

        let idle = Idle::action()
            .with_precondition(Idling::is(false))
            .with_mutator(Idling::set(true));

        let (planner, components) = create_planner!({
            actions: [
                (Rest, rest_action),
                (ServeOrder, serve_order_action),
                (ProduceLemonade, produce_lemonade_action),
                (GoToLemonadeMaker, go_to_lemonade_maker_action),
                (GoToOrderDesk, go_to_order_desk_action),
                (Idle, idle),
            ],
            state: [
                Energy(50.0),
                ServedOrder(false),
                AtLemonadeMaker(false),
                AtOrderDesk(false),
                CarryingItem(Item::Nothing),
                ShouldGoToOrderDesk(false),
                Idling(false),
            ],
            goals: [at_order_desk_goal, idle_goal],
        });

        commands
            .spawn((
                Agent,
                Name::new("Worker"),
                Visibility::default(),
                Worker,
                planner,
                components,
                Transform::from_xyz(0.0, 0.0, 1.0),
            ))
            .with_children(|subcommands| {
                subcommands.spawn((
                    Transform::from_translation(Vec3::new(10.0, -10.0, 10.0)),
                    Text2d("".into()),
                    TextFont {
                        font_size: 12.0,
                        ..default()
                    },
                    bevy::sprite::Anchor::TOP_LEFT,
                    StateDebugText,
                ));
            });
    }

    commands
        .spawn((
            Name::new("Lemonade Maker"),
            LemonadeMaker,
            Transform::from_xyz(100.0, 0.0, 1.0),
            Visibility::default(),
        ))
        .with_children(|subcommands| {
            subcommands.spawn((
                Transform::from_translation(Vec3::new(0.0, 25.0, 10.0)),
                Text2d("Lemonade Maker".into()),
                TextFont {
                    font_size: 12.0,
                    ..default()
                },
                bevy::sprite::Anchor::TOP_LEFT,
                StateDebugText,
            ));
        });

    commands
        .spawn((
            Name::new("Order Desk"),
            OrderDesk::default(),
            Transform::from_xyz(-100.0, 0.0, 1.0),
            Visibility::default(),
        ))
        .with_children(|subcommands| {
            subcommands.spawn((
                Transform::from_translation(Vec3::new(0.0, 50.0, 10.0)),
                Text2d("Order Desk".into()),
                TextFont {
                    font_size: 12.0,
                    ..default()
                },
                bevy::sprite::Anchor::TOP_LEFT,
                StateDebugText,
            ));
        });

    commands.spawn(Camera2d);
}

fn update_plan(planners: Query<Entity, With<Planner>>, mut commands: Commands) {
    for planner in planners.iter() {
        commands.entity(planner).trigger(UpdatePlan::from);
    }
}

fn handle_call_worker_to_empty_order_desk(
    mut q_order_desks: Query<&mut OrderDesk>,
    mut q_workers: Query<
        (Entity, &mut ShouldGoToOrderDesk),
        (With<Worker>, Without<GoToOrderDesk>),
    >,
    mut commands: Commands,
) {
    for mut order_desk in q_order_desks.iter_mut() {
        if order_desk.assigned_customer.is_some() && order_desk.assigned_worker.is_none() {
            // This order desk needs a worker!
            let (worker, mut should_go) = q_workers.iter_mut().next().expect("no workers");
            should_go.0 = true;
            order_desk.assigned_worker = Some(worker);
            commands.entity(worker).insert(GoToOrderDesk);
        }
    }
}

fn handle_idle(mut query: Query<(), With<Idle>>) {
    for _ in query.iter_mut() {
        // Don't set `Idling` to true: we leave this goal unatainable so it is always a valid fallback
        info!("I'm idling!");
    }
}

fn handle_move_to(
    mut commands: Commands,
    time: Res<Time>,
    mut query: Query<(Entity, &MoveTo, &mut Transform)>,
) {
    for (entity, move_to, mut transform) in query.iter_mut() {
        let destination = move_to.0;

        if transform.translation.distance(destination) > 5.0 {
            // If we're further away than 5 units, move closer
            let direction = (destination - transform.translation).normalize();
            transform.translation += direction * 96.0 * time.delta_secs();
        } else {
            // If we're within 5 units, assume the MoveTo completed
            commands.entity(entity).remove::<MoveTo>();
        }
    }
}

fn handle_go_to_order_desk(
    mut commands: Commands,
    mut q_order_desks: Query<(&Transform, &mut OrderDesk)>,
    mut query: Query<
        (Entity, &Transform, &mut AtOrderDesk, Has<Customer>),
        (With<GoToOrderDesk>, Without<MoveTo>),
    >,
) {
    for (entity, transform, mut state, customer) in query.iter_mut() {
        let (t_order_desk, mut order_desk) = q_order_desks
            .single_mut()
            .expect("Only one order desk expected!");

        // Offset to the left for customer, to the right for worker
        let with_offset = if customer {
            t_order_desk.translation + Vec3::new(-50.0, 0.0, 0.0)
        } else {
            t_order_desk.translation + Vec3::new(50.0, 0.0, 0.0)
        };

        let distance = with_offset.distance(transform.translation);

        if distance > 5.0 {
            commands.entity(entity).insert(MoveTo(with_offset));
        } else {
            state.0 = true;
            commands.entity(entity).remove::<GoToOrderDesk>();

            if customer {
                order_desk.assigned_customer = Some(entity);
            } else {
                order_desk.can_take_order = true;
            };
        }
    }
}

fn handle_wait_for_order(mut query: Query<&Customer, With<WaitForOrder>>, q_order: Query<&Order>) {
    for customer in query.iter_mut() {
        match customer.order {
            Some(e_order) => {
                let order = q_order.get(e_order).expect("Impossible!");
                if order.items_to_produce.is_empty() {
                    // ORder is ready! Destroy and move on
                } else {
                    // Order not yet ready
                }
            }
            None => {
                // Shouldn't be possible!
            }
        }
    }
}

fn handle_place_order(
    mut commands: Commands,
    time: Res<Time>,
    mut query: Query<(Entity, &mut Customer, &mut PlacedOrder), With<PlaceOrder>>,
    mut q_order_desks: Query<&mut OrderDesk>,
    mut progresses: Local<HashMap<Entity, Timer>>,
) {
    for (entity, mut customer, mut placed_order) in query.iter_mut() {
        let mut order_desk = q_order_desks
            .single_mut()
            .expect("Only one order desk expected!");
        // Need to make sure the serving counter has a worker at it before we
        // can place an order
        if order_desk.assigned_worker.is_some() && order_desk.can_take_order {
            match progresses.get_mut(&entity) {
                Some(progress) => {
                    if progress.tick(time.delta()).just_finished() {
                        info!("PlaceOrder complete!");
                        // Produce Order with one Lemonade, assign to OrderDesk
                        let new_order = Order {
                            items_to_produce: VecDeque::from([Item::Lemonade]),
                            _items: vec![],
                            _owner: entity,
                        };

                        let e_order = commands.spawn((Name::new("Order"), new_order)).id();
                        order_desk.current_order = Some(e_order);
                        customer.order = Some(e_order);

                        placed_order.0 = true;
                    } else {
                        // In progress...
                        info!("PlaceOrder Progress: {}", progress.fraction());
                    }
                }
                None => {
                    progresses.insert(entity, Timer::from_seconds(1.0, TimerMode::Once));
                }
            }
        }
    }
}

fn handle_pickup_lemonade(
    mut commands: Commands,
    time: Res<Time>,
    mut query: Query<
        (
            Entity,
            &mut CarryingItem,
            &mut OrderReady,
            &mut PlacedOrder,
            &mut AtOrderDesk,
        ),
        With<PickupLemonade>,
    >,
    mut progresses: Local<HashMap<Entity, Timer>>,
) {
    for (entity, mut state, mut order_ready, mut placed_order, mut at_order_desk) in
        query.iter_mut()
    {
        match progresses.get_mut(&entity) {
            Some(progress) => {
                if progress.tick(time.delta()).just_finished() {
                    state.0 = Item::Lemonade;

                    // Reset order status
                    order_ready.0 = false;
                    placed_order.0 = false;
                    at_order_desk.0 = false;

                    commands
                        .entity(entity)
                        .remove::<PickupLemonade>()
                        .insert(MoveTo(Vec3::new(-222.0, 0.0, 0.0)));

                    progresses.remove(&entity);
                } else {
                    // In progress...
                    info!("Pickup Progress: {}", progress.fraction());
                }
            }
            None => {
                progresses.insert(entity, Timer::from_seconds(1.0, TimerMode::Once));
            }
        }
    }
}

fn handle_drink_lemonade(
    mut commands: Commands,
    time: Res<Time>,
    mut query: Query<(Entity, &mut CarryingItem, &mut Thirst), With<DrinkLemonade>>,
    mut progresses: Local<HashMap<Entity, Timer>>,
) {
    for (entity, mut state, mut thirst) in query.iter_mut() {
        match progresses.get_mut(&entity) {
            Some(progress) => {
                if progress.tick(time.delta()).just_finished() {
                    state.0 = Item::Nothing;

                    commands.entity(entity).remove::<DrinkLemonade>();
                    progresses.remove(&entity);
                } else {
                    // In progress...
                    thirst.0 = (thirst.0 - 0.05).max(0.0);
                    info!("Drink Progress: {}", progress.fraction());
                }
            }
            None => {
                progresses.insert(entity, Timer::from_seconds(1.0, TimerMode::Once));
            }
        }
    }
}

fn update_thirst(time: Res<Time>, mut query: Query<&mut Thirst>) {
    for mut thirst in query.iter_mut() {
        thirst.0 += time.delta_secs_f64() * 0.3;
        if thirst.0 > 100.0 {
            thirst.0 = 100.0;
        }
    }
}

fn draw_state_debug(
    q_planners: Query<(Entity, &Name, &Children), With<Planner>>,
    q_actions: Query<(Entity, &dyn ActionComponent)>,
    q_datums: Query<(Entity, &dyn DatumComponent)>,
    q_child: Query<Entity, With<StateDebugText>>,
    mut text_writer: Text2dWriter,
) {
    for (entity, name, children) in q_planners.iter() {
        let mut current_action = "Idle";

        // Get current action, should always be one so grab the first one we find
        for (_entity, actions) in q_actions.get(entity).iter() {
            if let Some(action) = actions.iter().next() {
                current_action = action.action_type_name();
            }
        }

        // Concat all the found DatumComponents for this entity
        let mut state: String = "".to_string();
        for (_entity, data) in q_datums.get(entity).iter() {
            for datum in data.iter() {
                state = format!(
                    "{}\n{}: {}",
                    state,
                    datum.field_key(),
                    match datum.field_value() {
                        Datum::Bool(v) => v.to_string(),
                        Datum::F64(v) => format!("{v:.2}").to_string(),
                        Datum::I64(v) => format!("{v}").to_string(),
                        Datum::Enum(v) => format!("{v}").to_string(),
                    }
                );
            }
        }

        // Render it out
        for child in children.iter() {
            let text = q_child.get(child).unwrap();
            *text_writer.text(text, 0) =
                format!("{name}\n{current_action}\nEntity: {entity}\n---\n{state}");
        }
    }
}

fn draw_ui(
    mut gizmos: Gizmos,
    q_customer: Query<&Transform, With<Customer>>,
    q_workers: Query<&Transform, With<Worker>>,
    q_lemonade_makers: Query<&Transform, With<LemonadeMaker>>,
    q_order_desks: Query<&Transform, With<OrderDesk>>,
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

    for transform in q_customer.iter() {
        gizmos.circle_2d(transform.translation.xy(), 16., GREEN);
    }

    for transform in q_workers.iter() {
        gizmos.circle_2d(transform.translation.xy(), 16., BLUE);
    }

    for transform in q_lemonade_makers.iter() {
        gizmos.rect_2d(transform.translation.xy(), Vec2::new(20.0, 20.0), GOLD);
    }

    for transform in q_order_desks.iter() {
        gizmos.rect_2d(
            transform.translation.xy(),
            Vec2::new(40.0, 40.0),
            BLUE_VIOLET,
        );
    }
}
