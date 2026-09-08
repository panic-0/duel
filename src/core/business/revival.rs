//! 复活业务：救回数据实例与响应死亡前通知的 System。
//! 救回机会作为数据实例存在；成功消费与回血在同一次操作中显式关联。

use super::super::{
    buff_data::DestructionReason,
    event::{Event, EventType},
    log::LogEntry,
    operation::{
        completed, skipped, ChangeSet, ExecutionContext, Operation, OperationError,
        OperationOutcome,
    },
    query::Query,
    system::{Fact, NoticeKind, Priority, Subject, System},
    world::World,
    BuffId, PlayerId,
};

/// 救回数据实例：`player_id` 是业务上的服务对象（被救回的角色）；
/// 记录 owner 只表示生命周期依赖（随谁销毁），与救回目标无关。
#[derive(Debug)]
pub struct RevivalData {
    pub player_id: PlayerId,
}

/// 响应死亡前通知的救回 System：为每个服务于该玩家的救回实例产生候选。
/// 匹配使用业务字段 `player_id`；记录上的 owner 只决定随谁销毁。
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
            .filter(|(_, _, data)| data.player_id == *player_id)
            .map(|(id, _, _)| Subject::Instance(*id))
            .collect()
    }

    fn respond(
        &self,
        fact: &Fact<'_>,
        subject: Subject,
        query: &Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        let Some(Event::BeforePlayerDeath(event_player)) = fact.event() else {
            return Ok(Vec::new());
        };
        let Subject::Instance(buff_id) = subject else {
            return Ok(Vec::new());
        };
        let Some(data) = query.data::<RevivalData>(buff_id) else {
            return Ok(Vec::new());
        };
        // C3A：候选身份在收集时固定，但适用条件不冻结——
        // 同一通知中更早的响应可能已通过受控更新改变本实例的业务对象；
        // 轮到本候选时重新核对，不再匹配本次事件就跳过。
        if data.player_id != *event_player {
            return Ok(Vec::new());
        }
        Ok(vec![Box::new(RevivalOperation {
            source_id: data.player_id,
            buff_id,
        })])
    }
}

/// 一次救回：消费救回机会与恢复生命在同一次关联提交中完成，
/// 任何响应都不会看到“机会已消耗但生命仍为零”的半次提交。
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
        // 写入前验证：救回机会必须仍然存在，否则不免费治疗。
        if context.data::<RevivalData>(self.buff_id).is_none() {
            return skipped();
        }
        // 联合提交：消费机会（Consumed）与恢复生命一起写入，之后才开放通知。
        let changes = ChangeSet::new()
            .hp(player_id, amount as i64)
            .destroy(self.buff_id, DestructionReason::Consumed);
        let result = context.submit(changes)?;
        let revived =
            result.destroyed.iter().any(|d| d.buff_id == self.buff_id) && result.hp.is_some();
        if !revived {
            return skipped();
        }
        context.log(LogEntry::Revival { player_id });
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
