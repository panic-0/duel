use std::any::Any;
use std::collections::VecDeque;
use std::fmt::Debug;

use super::event::Checkpoint;
use super::flow::GameResult;
use super::{event::Event, state::GameState, world::World};

/// 操作返回的值。在执行器边界做类型擦除，让互不相关的操作共用同一个迭代工作栈。
pub type OperationValue = Box<dyn Any>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationError {
    Invalid(String),
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationResult {
    Completed,
    Skipped,
}

pub type OperationOutcome = (OperationResult, Option<OperationValue>);

pub fn completed() -> Result<OperationOutcome, OperationError> {
    Ok((OperationResult::Completed, None))
}

pub fn completed_with<T: Any>(value: T) -> Result<OperationOutcome, OperationError> {
    Ok((OperationResult::Completed, Some(Box::new(value))))
}

pub fn skipped() -> Result<OperationOutcome, OperationError> {
    Ok((OperationResult::Skipped, None))
}

/// 一段游戏行为。操作刻意保持小巧：复杂规则自己持有阶段状态，
/// 通过执行上下文调度子操作，而不是让执行器了解这条规则。
pub trait Operation: Debug + 'static {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError>;
}

impl Operation for Box<dyn Operation> {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        (*self).execute(context)
    }
}

pub(crate) trait ErasedOperation: Debug {
    fn execute_erased(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError>;
}

impl<T: Operation> ErasedOperation for T {
    fn execute_erased(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        self.execute(context)
    }
}

/// 交给操作的受限能力集合。世界本身不暴露，
/// 因此操作无法任意修改状态或重入驱动器。
pub struct ExecutionContext<'a> {
    world: &'a mut World,
    children: VecDeque<Box<dyn ErasedOperation>>,
}

pub type OperationContext<'a> = ExecutionContext<'a>;

impl<'a> ExecutionContext<'a> {
    pub(crate) fn new(world: &'a mut World) -> Self {
        Self {
            world,
            children: VecDeque::new(),
        }
    }

    pub fn state(&self) -> &GameState {
        self.world.state_view()
    }

    pub fn world(&self) -> &GameState {
        self.state()
    }

    /// 发布事件并等待其全部响应完成。错误记入世界后本调用继续返回；
    /// 后续受控入口（提交、子操作）会拒绝执行，根调用最终返回该错误。
    pub fn publish(&mut self, event: Event) {
        if let Err(error) = self.try_publish(event) {
            self.world.record_operation_error(error);
        }
    }

    /// 发布事件并等待其全部响应完成；事件是不可变事实，没有读回值。
    pub fn try_publish(&mut self, event: Event) -> Result<(), OperationError> {
        self.world.settle_operation_event(event)
    }

    /// 同步执行子操作；返回时，子操作及其事件响应均已完成。
    pub fn execute<O: Operation>(
        &mut self,
        operation: O,
    ) -> Result<OperationOutcome, OperationError> {
        self.world.execute_child(Box::new(operation))
    }

    /// 同步执行类型擦除后的子操作。
    pub fn execute_boxed(
        &mut self,
        operation: Box<dyn Operation>,
    ) -> Result<OperationOutcome, OperationError> {
        self.world.execute_child(Box::new(operation))
    }

    pub fn log(&self, entry: super::log::LogEntry) {
        self.world.log(entry);
    }

    pub fn is_end(&self) -> bool {
        self.world.is_end()
    }

    pub fn end_game(&mut self, result: GameResult) {
        self.world.end_game(result);
    }

    /// 查询该玩家在 `after`（已执行的最远技能槽位）之后的下一个正常行动。
    /// 每次调用都按当前状态重新收集；槽位是稳定身份，可用性变化不会让游标错位。
    pub fn normal_action_after(
        &mut self,
        player_id: super::PlayerId,
        after: Option<(super::BuffId, usize)>,
    ) -> Option<((super::BuffId, usize), Box<dyn Operation>)> {
        self.world.normal_action_after(player_id, after)
    }

    /// 提交一次生命修改并完成其事件响应。
    /// `Ok(None)` 表示目标不存在（可解释的跳过）；`Err` 表示响应链失败且已记录。
    pub fn modify_hp(
        &mut self,
        target_id: super::PlayerId,
        modifier: i64,
    ) -> Result<Option<HpChange>, OperationError> {
        self.world.modify_hp_from_operation(target_id, modifier)
    }

    /// 提交一次伤害（含参数修改与声明的消耗）并完成其事件响应。
    pub fn submit_damage(
        &mut self,
        damage: DamageContext,
    ) -> Result<Option<HpChange>, OperationError> {
        self.world.submit_damage(damage)
    }

    /// 提交一次治疗并完成其事件响应。
    pub fn submit_heal(
        &mut self,
        target_id: super::PlayerId,
        amount: u64,
    ) -> Result<Option<HpChange>, OperationError> {
        self.world.submit_heal(target_id, amount)
    }

    pub fn add_player(&mut self, player: super::player::Player) -> super::PlayerId {
        self.world.add_player(player)
    }

    pub fn remove_player(&mut self, id: super::PlayerId) -> Option<super::player::Player> {
        self.world.remove_player(id)
    }

    pub fn add_buff(&mut self, buff: Box<dyn super::buff::Buff>) -> super::BuffId {
        self.world.add_buff(buff)
    }

    pub fn remove_buff(&mut self, id: super::BuffId) -> Option<Box<dyn super::buff::Buff>> {
        self.world.remove_buff(id)
    }

    pub(crate) fn cleanup_owned_buffs(&mut self, player_id: super::PlayerId) {
        self.world.cleanup_owned_buffs(player_id);
    }

    pub fn spawn<O: Operation>(&mut self, operation: O) {
        self.children.push_back(Box::new(operation));
    }

    pub(crate) fn take_children(&mut self) -> VecDeque<Box<dyn ErasedOperation>> {
        std::mem::take(&mut self.children)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HpChange {
    pub target_id: super::PlayerId,
    pub old_hp: u64,
    pub new_hp: u64,
    pub max_hp: u64,
    pub requested: i64,
    /// 本次提交的无符号数值；伤害和治疗由调用的提交接口区分。
    pub submitted_amount: u64,
}

/// 一次伤害的参数草稿。修改回调只调整这里的数据并声明消耗，
/// 提交与消耗由执行器的受控提交统一处理。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DamageContext {
    pub source_id: Option<super::PlayerId>,
    pub target_id: super::PlayerId,
    pub amount: u64,
    /// 本次提交成功时需要消耗的机会（例如移除一次性护盾 Buff）。
    consumption: Vec<super::BuffId>,
}

impl DamageContext {
    pub fn new(
        source_id: Option<super::PlayerId>,
        target_id: super::PlayerId,
        amount: u64,
    ) -> Self {
        Self {
            source_id,
            target_id,
            amount,
            consumption: Vec::new(),
        }
    }

    pub fn reduce_to(&mut self, amount: u64) {
        self.amount = amount.min(self.amount);
    }

    /// 声明本次提交成功时消耗一次机会；执行器在提交后、通知反应前统一处理。
    pub fn consume_buff(&mut self, buff_id: super::BuffId) {
        self.consumption.push(buff_id);
    }

    pub(crate) fn take_consumption(&mut self) -> Vec<super::BuffId> {
        std::mem::take(&mut self.consumption)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Damage {
    pub context: DamageContext,
}

impl Damage {
    pub fn new(
        source_id: Option<super::PlayerId>,
        target_id: super::PlayerId,
        amount: u64,
    ) -> Self {
        Self {
            context: DamageContext::new(source_id, target_id, amount),
        }
    }
}

impl Operation for Damage {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        let Some(change) = context.submit_damage(self.context)? else {
            return skipped();
        };
        completed_with(change)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Heal {
    pub target_id: super::PlayerId,
    pub amount: u64,
}

impl Heal {
    pub fn new(target_id: super::PlayerId, amount: u64) -> Self {
        Self { target_id, amount }
    }
}

impl Operation for Heal {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        let Some(change) = context.submit_heal(self.target_id, self.amount)? else {
            return skipped();
        };
        completed_with(change)
    }
}

#[derive(Debug, Clone)]
pub struct EmitEvent(pub Event);

impl Operation for EmitEvent {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        context.publish(self.0);
        completed()
    }
}

#[derive(Debug)]
pub struct AddPlayerOperation(pub super::player::Player);

impl Operation for AddPlayerOperation {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        context.add_player(self.0);
        completed()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RemovePlayerOperation(pub super::PlayerId);

impl Operation for RemovePlayerOperation {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        if context.remove_player(self.0).is_some() {
            completed()
        } else {
            skipped()
        }
    }
}

#[derive(Debug)]
pub struct AddBuffOperation(pub Box<dyn super::buff::Buff>);

impl Operation for AddBuffOperation {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        context.add_buff(self.0);
        completed()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RemoveBuffOperation(pub super::BuffId);

impl Operation for RemoveBuffOperation {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        if context.remove_buff(self.0).is_some() {
            completed()
        } else {
            skipped()
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DeathOperation {
    pub player_id: super::PlayerId,
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

/// 发布胜负检查点；终局判断由全局胜负 Buff 在检查点上作出。
fn run_checkpoint(
    context: &mut ExecutionContext<'_>,
    phase: Checkpoint,
    round: Option<u32>,
) -> Result<(), OperationError> {
    context.try_publish(Event::Checkpoint { phase, round })
}

/// 一个完整回合：回合开始通知、按座次逐个执行玩家的 Turn、轮末通知与检查点。
/// 终局确认后不再补执行剩余的轮末效果。
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
    pub player_id: super::PlayerId,
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
        let mut executed: Option<(super::BuffId, usize)> = None;
        while !context.is_end() {
            let Some((slot, action)) = context.normal_action_after(self.player_id, executed) else {
                break;
            };
            context.execute_boxed(action)?;
            executed = Some(slot);
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

impl Operation for DuelOperation {
    fn execute(
        self: Box<Self>,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        context.try_publish(Event::DuelStart)?;
        run_checkpoint(context, Checkpoint::DuelStart, None)?;
        if self.max_rounds == 0 && !context.is_end() {
            context.end_game(GameResult::Draw);
        }

        let mut round = 1;
        while !context.is_end() && round <= self.max_rounds {
            context.execute(RoundOperation { round })?;
            if !context.is_end() && round >= self.max_rounds {
                context.end_game(GameResult::Draw);
            }
            round += 1;
        }
        completed()
    }
}

impl DeathOperation {
    /// 死亡确认：死亡前通知、按当前状态重新判断、提交与清理。
    fn settle(
        player_id: super::PlayerId,
        context: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        context.try_publish(Event::BeforePlayerDeath(player_id))?;
        let Some(player) = context.state().get_player(player_id) else {
            return skipped();
        };
        if player.hp() != 0 {
            return skipped();
        }
        context.log(super::log::LogEntry::Death { player_id });
        context.remove_player(player_id);
        context.try_publish(Event::AfterPlayerDeath(player_id))?;
        context.cleanup_owned_buffs(player_id);
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
        if context.world.pending_deaths.contains(&player_id) {
            return skipped();
        }
        let Some(player) = context.state().get_player(player_id) else {
            return skipped();
        };
        if player.hp() != 0 {
            return skipped();
        }
        context.world.pending_deaths.push(player_id);
        let outcome = Self::settle(player_id, context);
        context
            .world
            .pending_deaths
            .retain(|&pending| pending != player_id);
        outcome
    }
}
