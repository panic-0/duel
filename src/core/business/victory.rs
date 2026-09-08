//! 胜负业务：最后一人生还的独立胜负 System。
//! 在流程发布的检查点上按当前状态判断，不伪造空数据实例。

use super::super::{
    event::{Event, EventType},
    operation::{completed, ExecutionContext, Operation, OperationError, OperationOutcome},
    query::Query,
    system::{Fact, NoticeKind, Priority, Subject, System},
    world::GameResult,
};

/// 独立胜负 System：检查点上只剩一名玩家时结束对局。
#[derive(Debug)]
pub(crate) struct VictorySystem;

impl System for VictorySystem {
    fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
        vec![(NoticeKind::Event(EventType::Checkpoint), Priority::Final)]
    }

    fn candidates(&self, fact: &Fact<'_>, _query: &Query<'_>) -> Vec<Subject> {
        matches!(fact.event(), Some(Event::Checkpoint { .. }))
            .then(|| vec![Subject::Standalone])
            .unwrap_or_default()
    }

    fn respond(
        &self,
        _fact: &Fact<'_>,
        _subject: Subject,
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
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        if context.state().get_players().len() <= 1 {
            context.end_game(GameResult::LastStanding)?;
        }
        completed()
    }
}
