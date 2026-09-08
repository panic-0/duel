use super::super::{
    event::{Event, EventType},
    log::LogEntry,
    operation::{completed, skipped, ExecutionContext, Operation, OperationError, OperationResult},
    state::GameState,
    BuffId, PlayerId,
};
use super::{Buff, Priority};

#[derive(Debug)]
pub struct Revival {
    pub source_id: PlayerId,
}

#[derive(Debug, Clone, Copy)]
pub struct RevivalOperation {
    pub source_id: PlayerId,
    pub buff_id: BuffId,
}

impl Operation for RevivalOperation {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<(OperationResult, Option<Box<dyn std::any::Any>>), OperationError> {
        let Some(player) = context.state().get_player(self.source_id) else {
            return skipped();
        };
        if player.hp() != 0 {
            return skipped();
        }
        let amount = player.max_hp() / 2;
        if amount == 0 {
            return skipped();
        }
        // 先成功消费救回机会，再恢复生命；机会已失效就不免费治疗。
        if context.remove_buff(self.buff_id).is_none() {
            return skipped();
        }
        context.log(LogEntry::Revival {
            player_id: self.source_id,
        });
        context.modify_hp(self.source_id, amount as i64)?;
        completed()
    }
}

impl Revival {
    pub fn new(source_id: PlayerId) -> Self {
        Revival { source_id }
    }
}

impl Buff for Revival {
    fn owner(&self) -> Option<PlayerId> {
        Some(self.source_id)
    }
    fn subscriptions(&self) -> Vec<(EventType, Priority)> {
        vec![(EventType::BeforePlayerDeath, Priority::Default)]
    }

    fn operations(
        &mut self,
        event: &Event,
        _world: &GameState,
        buff_id: BuffId,
    ) -> Vec<Box<dyn Operation>> {
        match *event {
            Event::BeforePlayerDeath(player_id) if player_id == self.source_id => {
                vec![Box::new(RevivalOperation {
                    source_id: self.source_id,
                    buff_id,
                })]
            }
            _ => Vec::new(),
        }
    }
}
