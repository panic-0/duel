//! 复活业务：救回数据实例与响应死亡前通知的 System。
//! 救回机会作为数据实例存在；成功消费与回血在同一次操作中显式关联。

use super::super::{
    buff_data::DestructionReason,
    event::{Event, EventType},
    log::LogEntry,
    operation::{
        completed, skipped, ExecutionContext, Operation, OperationError, OperationOutcome,
    },
    query::Query,
    system::{Fact, NoticeKind, Priority, Subject, System},
    world::World,
    BuffId, PlayerId,
};

/// 救回数据实例：`owner` 随玩家销毁；消费时机由救回操作决定。
#[derive(Debug)]
pub struct RevivalData {
    pub player_id: PlayerId,
}

/// 响应死亡前通知的救回 System：为每个属于该玩家的救回实例产生候选。
#[derive(Debug)]
pub struct RevivalSystem;

impl System for RevivalSystem {
    fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
        vec![(
            NoticeKind::Event(EventType::BeforePlayerDeath),
            Priority::Default,
        )]
    }

    fn candidates(&self, fact: &Fact<'_>, query: &Query<'_>) -> Vec<Subject> {
        let Some(Event::BeforePlayerDeath(player_id)) = fact.event() else {
            return Vec::new();
        };
        query
            .instances::<RevivalData>()
            .iter()
            .filter(|(_, owner, _)| *owner == Some(*player_id))
            .map(|(id, _, _)| Subject::Instance(*id))
            .collect()
    }

    fn respond(
        &self,
        _fact: &Fact<'_>,
        subject: Subject,
        query: &Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        let Subject::Instance(buff_id) = subject else {
            return Ok(Vec::new());
        };
        let Some(data) = query.data::<RevivalData>(buff_id) else {
            return Ok(Vec::new());
        };
        Ok(vec![Box::new(RevivalOperation {
            source_id: data.player_id,
            buff_id,
        })])
    }
}

/// 一次救回：先成功消费救回机会，再恢复生命；机会已失效就不免费治疗。
#[derive(Debug)]
pub struct RevivalOperation {
    source_id: PlayerId,
    buff_id: BuffId,
}

impl Operation for RevivalOperation {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        let player_id = self.source_id;
        let Some(player) = context.state().get_player(player_id) else {
            return skipped();
        };
        if player.hp() != 0 {
            return skipped();
        }
        let amount = player.max_hp() / 2;
        if amount == 0 {
            return skipped();
        }
        if !context.remove_data(self.buff_id, DestructionReason::Consumed)? {
            return skipped();
        }
        context.log(LogEntry::Revival { player_id });
        context.modify_hp(player_id, amount as i64)?;
        completed()
    }
}

/// 注册救回 System（一次即可）；System 的注册与实例生命周期独立。
pub fn register_revival_system(world: &mut World) {
    world.add_system(RevivalSystem);
}

/// 为一名玩家添加一次救回机会，返回实例身份。
pub fn add_revival(world: &mut World, player_id: PlayerId) -> BuffId {
    world.add_data(Some(player_id), RevivalData { player_id })
}
