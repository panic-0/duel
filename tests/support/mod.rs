//! 多个行为测试共用的事件与销毁探针、日志收集工具。

use duel::core::{
    event::{Checkpoint, Event, EventKind},
    log::{LogEntry, Logger},
    operation::{completed, ActionContext, Operation, OperationError, OperationOutcome},
    query::Query,
    state::BattleState,
    system::{EventEnvelope, Priority, ReactionTarget, System},
    BattleEngine, PlayerId,
};
use std::{
    cell::{Cell, RefCell},
    fmt,
    rc::Rc,
};

pub(crate) fn log_sink() -> (Rc<RefCell<Vec<LogEntry>>>, Logger) {
    let entries = Rc::new(RefCell::new(Vec::new()));
    let sink = entries.clone();
    let logger = Logger::new(Box::new(move |_world, entry| {
        sink.borrow_mut().push(entry.clone());
    }));
    (entries, logger)
}

pub(crate) fn count(entries: &[LogEntry], pred: impl Fn(&LogEntry) -> bool) -> usize {
    entries.iter().filter(|e| pred(e)).count()
}

pub(crate) fn checkpoint() -> Event {
    Event::Checkpoint {
        phase: Checkpoint::ActionEnd,
        round: Some(1),
    }
}

#[derive(Debug)]
pub(crate) struct CountOperation(pub(crate) Rc<Cell<usize>>);

impl Operation for CountOperation {
    fn execute(
        self: Box<Self>,
        _: &mut ActionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        self.0.set(self.0.get() + 1);
        completed()
    }
}

#[derive(Debug)]
pub(crate) struct CheckpointCounter(pub(crate) Rc<Cell<usize>>);

impl System for CheckpointCounter {
    fn subscriptions(&self) -> Vec<(EventKind, Priority)> {
        vec![(EventKind::Checkpoint, Priority::Default)]
    }
    fn candidates(
        &self,
        fact: &EventEnvelope<'_>,
        _query: &duel::core::query::Query<'_>,
    ) -> Vec<ReactionTarget> {
        matches!(fact.event(), Some(Event::Checkpoint { .. }))
            .then(|| vec![ReactionTarget::Standalone])
            .unwrap_or_default()
    }
    fn respond(
        &self,
        _fact: &EventEnvelope<'_>,
        _subject: ReactionTarget,
        _query: &duel::core::query::Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        Ok(vec![Box::new(CountOperation(self.0.clone()))])
    }
}

#[derive(Debug)]
pub(crate) struct Count(pub(crate) Rc<Cell<usize>>);

impl Operation for Count {
    fn execute(
        self: Box<Self>,
        _: &mut ActionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        self.0.set(self.0.get() + 1);
        completed()
    }
}

/// 排序探针数据：每个实例一个身份编号。
#[derive(Debug)]
pub(crate) struct OrderMark(pub(crate) u8);

/// 减伤数据与规则（业务目标字段决定作用范围，owner 只管寿命）。
#[derive(Debug)]
pub(crate) struct CrossRoleShield {
    pub(crate) target_id: PlayerId,
}

#[derive(Debug)]
pub(crate) struct BelovedMark;

/// 在死亡后通知里统计依赖实例数量的探针。
#[derive(Debug)]
pub(crate) struct DependencyCounter {
    pub(crate) after_death_count: Rc<Cell<usize>>,
    pub(crate) observed_instances: Rc<Cell<i64>>,
}

impl System for DependencyCounter {
    fn subscriptions(&self) -> Vec<(EventKind, Priority)> {
        vec![(EventKind::AfterPlayerDeath, Priority::Default)]
    }
    fn candidates(&self, fact: &EventEnvelope<'_>, _query: &Query<'_>) -> Vec<ReactionTarget> {
        matches!(fact.event(), Some(Event::AfterPlayerDeath(_)))
            .then(|| vec![ReactionTarget::Standalone])
            .unwrap_or_default()
    }
    fn respond(
        &self,
        _fact: &EventEnvelope<'_>,
        _subject: ReactionTarget,
        query: &Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        self.after_death_count.set(self.after_death_count.get() + 1);
        self.observed_instances.set(
            query.components::<BelovedMark>().len() as i64
                + query.components::<CrossRoleShield>().len() as i64,
        );
        Ok(vec![])
    }
}

#[derive(Debug)]
pub(crate) struct RuntimeFailureOn(pub(crate) EventKind);

impl System for RuntimeFailureOn {
    fn subscriptions(&self) -> Vec<(EventKind, Priority)> {
        vec![(self.0, Priority::Default)]
    }
    fn candidates(&self, fact: &EventEnvelope<'_>, _query: &Query<'_>) -> Vec<ReactionTarget> {
        fact.event()
            .map(|_| vec![ReactionTarget::Standalone])
            .unwrap_or_default()
    }
    fn respond(
        &self,
        _fact: &EventEnvelope<'_>,
        _subject: ReactionTarget,
        _query: &Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        Err(OperationError::Failed("终局测试不应触发".into()))
    }
}

pub(crate) type Handler = dyn Fn(&EventEnvelope<'_>, &BattleState) -> Vec<Box<dyn Operation>>;

/// 通用响应 System：候选为独立响应，handler 决定提出的操作。
pub(crate) struct OpSystem {
    pub(crate) subscriptions: Vec<(EventKind, Priority)>,
    pub(crate) handler: Box<Handler>,
}

impl fmt::Debug for OpSystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("OpSystem")
    }
}

impl System for OpSystem {
    fn subscriptions(&self) -> Vec<(EventKind, Priority)> {
        self.subscriptions.clone()
    }

    fn candidates(
        &self,
        _fact: &EventEnvelope<'_>,
        _query: &duel::core::query::Query<'_>,
    ) -> Vec<ReactionTarget> {
        vec![ReactionTarget::Standalone]
    }

    fn respond(
        &self,
        fact: &EventEnvelope<'_>,
        _subject: ReactionTarget,
        query: &duel::core::query::Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        Ok((self.handler)(fact, query.state()))
    }
}

pub(crate) fn op_system(
    world: &mut BattleEngine,
    events: &[EventKind],
    handler: impl Fn(&EventEnvelope<'_>, &BattleState) -> Vec<Box<dyn Operation>> + 'static,
) {
    world.register_system(OpSystem {
        subscriptions: events
            .iter()
            .map(|&event| (event, Priority::Default))
            .collect(),
        handler: Box::new(handler),
    });
}

pub(crate) fn trace_events(world: &mut BattleEngine) -> Rc<RefCell<Vec<Event>>> {
    let trace = Rc::new(RefCell::new(Vec::new()));
    let sink = trace.clone();
    op_system(
        world,
        &[
            EventKind::DuelStart,
            EventKind::RoundStart,
            EventKind::BeforeTurn,
            EventKind::Turn,
            EventKind::AfterTurn,
            EventKind::RoundEnd,
            EventKind::BeforePlayerAttack,
            EventKind::PlayerAttack,
            EventKind::AfterPlayerAttack,
            EventKind::BeforePlayerDeath,
            EventKind::AfterPlayerDeath,
        ],
        move |fact, _state| {
            if let Some(event) = fact.event() {
                sink.borrow_mut().push(event.clone());
            }
            vec![]
        },
    );
    trace
}
