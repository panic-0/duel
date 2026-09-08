//! 死亡业务：死亡判断 System 与 Death 操作。
//! 死亡条件、救回窗口与正式死亡提交都由本模块的规则决定，
//! 基础生命周期机制只负责在死亡提交中同步销毁 owner 依赖。

use super::super::{
    event::{Event, EventType},
    log::LogEntry,
    operation::{
        completed, skipped, ChangeSet, ExecutionContext, Operation, OperationError,
        OperationOutcome, SubmissionResult,
    },
    query::Query,
    system::{Fact, NoticeKind, Priority, Subject, System},
    PlayerId,
};

/// 死亡进行中标记：业务状态，经中性扩展存储持有；
/// 同一次死亡不重复进入死亡前流程，正常返回与错误路径都会解除标记。
#[derive(Debug, Default)]
pub(crate) struct DeathGuard {
    pending: Vec<PlayerId>,
}

/// 正式死亡操作：死亡前通知、按当前状态重新判断、
/// 提交“移除角色 + 销毁 owner 依赖”，随后发布死亡后通知。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeathOperation {
    pub player_id: PlayerId,
}

impl DeathOperation {
    fn settle(
        player_id: PlayerId,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        // 死亡前通知：救回等响应完整结束后，再按当前状态重新判断。
        context.try_publish(Event::BeforePlayerDeath(player_id))?;
        if context.state().get_player(player_id).is_none() {
            return completed();
        }
        if context
            .state()
            .get_player(player_id)
            .is_some_and(|p| p.hp() != 0)
        {
            return completed();
        }
        context.log(LogEntry::Death { player_id });
        // 关联提交：移除角色与依赖销毁同边界完成，期间不运行玩法响应；
        // 提交内部按“销毁事实”顺序通知，完成后本操作再发布死亡后语义事件。
        let SubmissionResult { player_removed, .. } =
            context.submit(ChangeSet::new().remove_player(player_id))?;
        if player_removed {
            context.try_publish(Event::AfterPlayerDeath(player_id))?;
        }
        completed()
    }
}

impl Operation for DeathOperation {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        let player_id = self.player_id;
        // 同一次死亡只进入一次死亡前流程；嵌套的重复请求直接跳过。
        if context
            .resource_mut_or_default::<DeathGuard>()?
            .pending
            .contains(&player_id)
        {
            return skipped();
        }
        let Some(player) = context.state().get_player(player_id) else {
            return skipped();
        };
        if player.hp() != 0 {
            return skipped();
        }
        context
            .resource_mut_or_default::<DeathGuard>()?
            .pending
            .push(player_id);
        let outcome = Self::settle(player_id, context);
        // 进行中标记必须在错误路径上同样释放：
        // 这是必要的运行时收尾，使用失败后的受限清理路径，不承载新的玩法变化。
        if let Some(guard) = context.resource_mut_ignoring_failure::<DeathGuard>() {
            guard.pending.retain(|&pending| pending != player_id);
        }
        outcome
    }
}

/// 独立死亡判断 System：观察到存在零血角色的 HpChanged 后提出 Death。
#[derive(Debug)]
pub(crate) struct DeathSystem;

impl System for DeathSystem {
    fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
        vec![(NoticeKind::Event(EventType::HpChanged), Priority::Final)]
    }

    fn candidates(&self, fact: &Fact<'_>, query: &Query<'_>) -> Vec<Subject> {
        if fact.event().is_none() {
            return Vec::new();
        }
        if query
            .state()
            .get_players()
            .values()
            .any(|player| player.hp() == 0)
        {
            vec![Subject::Standalone]
        } else {
            Vec::new()
        }
    }

    fn respond(
        &self,
        _fact: &Fact<'_>,
        _subject: Subject,
        query: &Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        Ok(query
            .state()
            .get_players()
            .iter()
            .filter(|(_, player)| player.hp() == 0)
            .map(|(&id, _)| Box::new(DeathOperation { player_id: id }) as Box<dyn Operation>)
            .collect())
    }
}
