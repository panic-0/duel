//! 流程业务：Duel／Round／Turn 是普通 Operation，拥有自己的流程规则。
//! 每个正常行动及其全部反应完成后发布胜负检查点；终局判断由胜负 System 作出。

use super::super::{
    event::{Checkpoint, Event},
    operation::{completed, ExecutionContext, Operation, OperationError, OperationOutcome},
    world::{GameResult, World, MAX_ROUNDS},
    BuffId, PlayerId,
};
use super::skills::Abilities;

/// 完整跑一局：执行 DuelOperation 并返回引擎错误。
/// 默认规则需要先显式调用 `install_default_rules` 装配。
pub trait DuelRunner {
    fn run(&mut self) -> Result<(), OperationError>;
    fn run_with_max_rounds(&mut self, max_rounds: u32) -> Result<(), OperationError>;
}

impl DuelRunner for World {
    fn run(&mut self) -> Result<(), OperationError> {
        self.execute(DuelOperation::new(MAX_ROUNDS)).map(|_| ())
    }

    fn run_with_max_rounds(&mut self, max_rounds: u32) -> Result<(), OperationError> {
        self.execute(DuelOperation::new(max_rounds)).map(|_| ())
    }
}

/// 发布胜负检查点；终局判断由独立胜负 System 在检查点上作出。
fn run_checkpoint(
    context: &mut ExecutionContext<'_>,
    phase: Checkpoint,
    round: Option<u32>,
) -> Result<(), OperationError> {
    context.try_publish(Event::Checkpoint { phase, round })
}

/// 一个完整回合：回合开始通知、按座次逐个执行玩家的 Turn、轮末通知与检查点。
/// 正式终局后不再补执行剩余的轮末效果。
#[derive(Debug, Clone, Copy)]
pub struct RoundOperation {
    pub round: u32,
}

impl Operation for RoundOperation {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        context.try_publish(Event::RoundStart { round: self.round })?;
        run_checkpoint(context, Checkpoint::RoundStart, Some(self.round))?;
        if context.is_end() {
            return completed();
        }
        let mut current = context
            .state()
            .get_players()
            .iter()
            .find(|(_, player)| player.is_alive())
            .map(|(id, _)| *id);
        while let Some(player_id) = current {
            context.execute(TurnOperation {
                round: self.round,
                player_id,
            })?;
            if context.is_end() {
                break;
            }
            current = context.state().get_next_player_not_around(player_id);
        }
        if context.is_end() {
            return completed();
        }
        context.try_publish(Event::RoundEnd { round: self.round })?;
        run_checkpoint(context, Checkpoint::RoundEnd, Some(self.round))?;
        completed()
    }
}

/// 一名玩家的一次正常回合：回合通知与反应、逐个正常行动、回合收尾。
///
/// 正常行动由本流程按当前状态逐个向该玩家的技能集合查询并等待完成；
/// 游标是稳定的技能槽位，技能中途失效不会让后续技能被跳过。
/// 每个行动及其全部反应完成后发布 ActionEnd 检查点，已终局则不再安排后续行动。
#[derive(Debug, Clone, Copy)]
pub struct TurnOperation {
    pub round: u32,
    pub player_id: PlayerId,
}

impl Operation for TurnOperation {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        context.try_publish(Event::BeforeTurn {
            round: self.round,
            player_id: self.player_id,
        })?;
        run_checkpoint(context, Checkpoint::TurnStart, Some(self.round))?;
        if context.is_end() {
            return completed();
        }
        if !context
            .state()
            .get_player(self.player_id)
            .is_some_and(|player| player.is_alive())
        {
            return completed();
        }
        context.try_publish(Event::Turn {
            round: self.round,
            player_id: self.player_id,
        })?;
        // 游标记录“已执行的最远技能槽位”，保证资格变化不错位。
        let mut executed: Option<(BuffId, usize)> = None;
        while !context.is_end() {
            let Some(next) = self.next_action(context, executed) else {
                break;
            };
            let (key, action) = next;
            context.execute_boxed(action)?;
            executed = Some(key);
            run_checkpoint(context, Checkpoint::ActionEnd, Some(self.round))?;
        }
        if context.is_end() {
            return completed();
        }
        context.try_publish(Event::AfterTurn {
            round: self.round,
            player_id: self.player_id,
        })?;
        run_checkpoint(context, Checkpoint::TurnEnd, Some(self.round))?;
        completed()
    }
}

impl TurnOperation {
    /// 查询该玩家在游标之后的下一个正常行动；每次都按当前状态重新收集。
    fn next_action(
        &self,
        context: &mut ExecutionContext<'_>,
        after: Option<(BuffId, usize)>,
    ) -> Option<((BuffId, usize), Box<dyn Operation>)> {
        let mut chosen: Option<((BuffId, usize), Box<dyn Operation>)> = None;
        {
            let query = context.query();
            for (id, owner, abilities) in query.instances::<Abilities>() {
                if owner != Some(self.player_id) {
                    continue;
                }
                for (slot, action) in abilities.normal_actions(self.player_id, query.state()) {
                    let key = (id, slot);
                    if after.is_some_and(|previous| key <= previous) {
                        continue;
                    }
                    if chosen.as_ref().is_none_or(|(best, _)| key < *best) {
                        chosen = Some((key, action));
                    }
                }
            }
        }
        chosen
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DuelOperation {
    pub max_rounds: u32,
}

impl DuelOperation {
    pub fn new(max_rounds: u32) -> Self {
        Self { max_rounds }
    }
}

impl Operation for DuelOperation {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        context.try_publish(Event::DuelStart)?;
        run_checkpoint(context, Checkpoint::DuelStart, None)?;
        if self.max_rounds == 0 && !context.is_end() {
            context.end_game(GameResult::Draw)?;
        }

        let mut round = 1;
        while !context.is_end() && round <= self.max_rounds {
            context.execute(RoundOperation { round })?;
            if !context.is_end() && round >= self.max_rounds {
                context.end_game(GameResult::Draw)?;
            }
            round += 1;
        }
        completed()
    }
}
