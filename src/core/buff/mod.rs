pub mod revival;
pub use revival::Revival;

use super::{
    command::Commands,
    event::{Event, EventType},
    world::World,
    PlayerId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BuffType {
    Abilities,

    AttackSettlement,

    Revival,
}

pub fn get_event_priorities(event_type: EventType) -> &'static [BuffType] {
    match event_type {
        // State events
        EventType::DuelStart => &[],
        EventType::RoundStart => &[],
        EventType::BeforeTurn => &[],
        EventType::Turn => &[BuffType::Abilities],
        EventType::AfterTurn => &[],
        EventType::RoundEnd => &[],

        // Action events
        EventType::BeforePlayerAttack => &[],
        EventType::PlayerAttack => &[BuffType::AttackSettlement],
        EventType::AfterPlayerAttack => &[],
        EventType::BeforePlayerDeath => &[BuffType::Revival],
        EventType::AfterPlayerDeath => &[],
    }
}

pub trait Buff: std::fmt::Debug {
    fn buff_type(&self) -> BuffType;
    fn on_event(
        &self,
        event: &mut Event,
        world: &World,
        commands: &mut Commands,
        buff_id: super::BuffId,
    );
}
