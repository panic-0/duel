//! 攻击业务：普攻 Operation。组织前置通知、资格复查与 Damage 子操作。

use super::super::{
    event::Event,
    log::LogEntry,
    operation::{skipped, ExecutionContext, Operation, OperationError, OperationResult},
    state::GameState,
    PlayerId,
};
use super::damage::Damage;

#[derive(Debug)]
pub struct Attack;

#[derive(Debug, Clone, Copy)]
pub struct AttackOperation {
    pub source_id: PlayerId,
}

impl AttackOperation {
    pub fn new(source_id: PlayerId) -> Self {
        Self { source_id }
    }
}

/// 默认普攻规则：来源与目标在伤害提交前都有效，攻击才继续。
fn combatants_valid(
    context: &ExecutionContext<'_>,
    source_id: PlayerId,
    target_id: PlayerId,
) -> bool {
    context
        .state()
        .get_player(source_id)
        .is_some_and(|p| p.is_alive())
        && context
            .state()
            .get_player(target_id)
            .is_some_and(|p| p.is_alive())
}

impl Operation for AttackOperation {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<(OperationResult, Option<Box<dyn std::any::Any>>), OperationError> {
        let Some(source) = context.state().get_player(self.source_id) else {
            return skipped();
        };
        if !source.is_alive() {
            return skipped();
        }
        let Some(target_id) = context.state().get_next_player_around(self.source_id) else {
            return skipped();
        };
        context.try_publish(Event::BeforePlayerAttack {
            source_id: self.source_id,
            target_id,
        })?;
        // 攻击开始反应（陷阱、救回等）完成后，按当前状态重新检查双方资格。
        if !combatants_valid(context, self.source_id, target_id) {
            return Ok((OperationResult::Skipped, None));
        }
        let amount = context
            .state()
            .get_player(self.source_id)
            .map(|p| p.attack())
            .unwrap_or(0);
        context.log(LogEntry::Attack {
            source_id: self.source_id,
            target_id,
        });
        context.try_publish(Event::PlayerAttack {
            source_id: self.source_id,
            target_id,
            damage: amount,
        })?;
        // 最后一个提交前反应点：PlayerAttack 的响应仍可能改变资格。
        if !combatants_valid(context, self.source_id, target_id) {
            return Ok((OperationResult::Skipped, None));
        }
        let damage = Damage::new(Some(self.source_id), target_id, amount);
        let (_, value) = context.execute(damage)?;
        // 没有提交结果就不伪造成功伤害通知。
        let Some(change) = value
            .as_deref()
            .and_then(|value| value.downcast_ref::<super::super::operation::HpChange>())
            .copied()
        else {
            return Ok((OperationResult::Skipped, None));
        };
        context.try_publish(Event::AfterPlayerAttack {
            source_id: self.source_id,
            target_id,
            damage: change.submitted_amount,
        })?;
        Ok((OperationResult::Completed, Some(Box::new(change))))
    }
}

impl super::super::business::skills::Ability for Attack {
    fn operation(&self, source_id: PlayerId, _world: &GameState) -> Option<Box<dyn Operation>> {
        Some(Box::new(AttackOperation::new(source_id)))
    }
}
