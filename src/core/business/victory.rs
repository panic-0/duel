//! 胜负业务：最后一人生还的独立胜负 System。
//! 在流程发布的检查点上按当前状态判断，不伪造空数据实例。

use super::super::{
    engine::BattleResult,
    event::{Event, EventKind},
    operation::{completed, ActionContext, Operation, OperationError, OperationOutcome},
    query::Query,
    system::{EventEnvelope, Priority, ReactionTarget, System},
};

/// 独立胜负 System：检查点上只剩一名玩家时结束对局。
#[derive(Debug)]
pub(crate) struct VictorySystem;

impl System for VictorySystem {
    fn subscriptions(&self) -> Vec<(EventKind, Priority)> {
        vec![(EventKind::Checkpoint, Priority::Final)]
    }

    fn candidates(&self, fact: &EventEnvelope<'_>, _query: &Query<'_>) -> Vec<ReactionTarget> {
        matches!(fact.event(), Some(Event::Checkpoint { .. }))
            .then(|| vec![ReactionTarget::Standalone])
            .unwrap_or_default()
    }

    fn respond(
        &self,
        _fact: &EventEnvelope<'_>,
        _subject: ReactionTarget,
        _query: &Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        Ok(vec![Box::new(EndLastStanding)])
    }
}

#[derive(Debug)]
struct EndLastStanding;

impl Operation for EndLastStanding {
    fn execute(
        self: Box<Self>,
        context: &mut ActionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        if context.state().players().len() <= 1 {
            context.end_game(BattleResult::LastStanding)?;
        }
        completed()
    }
}
