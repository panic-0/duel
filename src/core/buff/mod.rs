pub mod revival;
pub use revival::Revival;

pub mod damage_reduction;
pub use damage_reduction::DamageReduction;

pub mod state_machine;
pub use state_machine::StateMachine;

use super::{
    command::Commands,
    event::{Event, EventType},
    world::World,
    PlayerId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BuffType {
    // World Buffs
    StateMachine,

    // Player Buffs
    Abilities,

    Attack,

    DamageReduction,
    Revival,
}

pub fn get_event_priorities(event_type: EventType) -> &'static [BuffType] {
    match event_type {
        // State events - 状态机在最后处理，用于生成下一个状态
        EventType::DuelStart => &[BuffType::StateMachine],
        EventType::RoundStart => &[BuffType::StateMachine],
        EventType::BeforeTurn => &[BuffType::StateMachine],
        EventType::Turn => &[BuffType::Abilities, BuffType::StateMachine],
        EventType::AfterTurn => &[BuffType::StateMachine],
        EventType::RoundEnd => &[BuffType::StateMachine],

        // Action events
        EventType::BeforePlayerAttack => &[BuffType::Attack],
        EventType::PlayerAttack => &[BuffType::DamageReduction, BuffType::Attack],
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
