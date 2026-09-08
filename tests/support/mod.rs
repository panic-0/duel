//! 多个行为测试共用的事件与销毁探针、日志收集工具。

use duel::core::{
    event::{Checkpoint, Event, EventType},
    log::{LogEntry, Logger},
    operation::{completed, ExecutionContext, Operation, OperationError, OperationOutcome},
    query::Query,
    state::GameState,
    system::{Fact, NoticeKind, Priority, Subject, System},
    PlayerId, World,
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
        _: &mut ExecutionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        self.0.set(self.0.get() + 1);
        completed()
    }
}

#[derive(Debug)]
pub(crate) struct CheckpointCounter(pub(crate) Rc<Cell<usize>>);

impl System for CheckpointCounter {
    fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
        vec![(NoticeKind::Event(EventType::Checkpoint), Priority::Default)]
    }
    fn candidates(&self, fact: &Fact<'_>, _query: &duel::core::query::Query<'_>) -> Vec<Subject> {
        matches!(fact.event(), Some(Event::Checkpoint { .. }))
            .then(|| vec![Subject::Standalone])
            .unwrap_or_default()
    }
    fn respond(
        &self,
        _fact: &Fact<'_>,
        _subject: Subject,
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
        _: &mut ExecutionContext<'_>,
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
    fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
        vec![(
            NoticeKind::Event(EventType::AfterPlayerDeath),
            Priority::Default,
        )]
    }
    fn candidates(&self, fact: &Fact<'_>, _query: &Query<'_>) -> Vec<Subject> {
        matches!(fact.event(), Some(Event::AfterPlayerDeath(_)))
            .then(|| vec![Subject::Standalone])
            .unwrap_or_default()
    }
    fn respond(
        &self,
        _fact: &Fact<'_>,
        _subject: Subject,
        query: &Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        self.after_death_count.set(self.after_death_count.get() + 1);
        self.observed_instances.set(
            query.instances::<BelovedMark>().len() as i64
                + query.instances::<CrossRoleShield>().len() as i64,
        );
        Ok(vec![])
    }
}

#[derive(Debug)]
pub(crate) struct RuntimeFailureOn(pub(crate) EventType);

impl System for RuntimeFailureOn {
    fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
        vec![(NoticeKind::Event(self.0), Priority::Default)]
    }
    fn candidates(&self, fact: &Fact<'_>, _query: &Query<'_>) -> Vec<Subject> {
        fact.event()
            .map(|_| vec![Subject::Standalone])
            .unwrap_or_default()
    }
    fn respond(
        &self,
        _fact: &Fact<'_>,
        _subject: Subject,
        _query: &Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        Err(OperationError::Failed("终局测试不应触发".into()))
    }
}

pub(crate) type Handler = dyn Fn(&Fact<'_>, &GameState) -> Vec<Box<dyn Operation>>;

/// 通用响应 System：候选为独立响应，handler 决定提出的操作。
pub(crate) struct OpSystem {
    pub(crate) subscriptions: Vec<(NoticeKind, Priority)>,
    pub(crate) handler: Box<Handler>,
}

impl fmt::Debug for OpSystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("OpSystem")
    }
}

impl System for OpSystem {
    fn subscriptions(&self) -> Vec<(NoticeKind, Priority)> {
        self.subscriptions.clone()
    }

    fn candidates(&self, _fact: &Fact<'_>, _query: &duel::core::query::Query<'_>) -> Vec<Subject> {
        vec![Subject::Standalone]
    }

    fn respond(
        &self,
        fact: &Fact<'_>,
        _subject: Subject,
        query: &duel::core::query::Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        Ok((self.handler)(fact, query.state()))
    }
}

pub(crate) fn op_system(
    world: &mut World,
    events: &[EventType],
    handler: impl Fn(&Fact<'_>, &GameState) -> Vec<Box<dyn Operation>> + 'static,
) {
    world.add_system(OpSystem {
        subscriptions: events
            .iter()
            .map(|&event| (NoticeKind::Event(event), Priority::Default))
            .collect(),
        handler: Box::new(handler),
    });
}

pub(crate) fn trace_events(world: &mut World) -> Rc<RefCell<Vec<Event>>> {
    let trace = Rc::new(RefCell::new(Vec::new()));
    let sink = trace.clone();
    op_system(
        world,
        &[
            EventType::DuelStart,
            EventType::RoundStart,
            EventType::BeforeTurn,
            EventType::Turn,
            EventType::AfterTurn,
            EventType::RoundEnd,
            EventType::BeforePlayerAttack,
            EventType::PlayerAttack,
            EventType::AfterPlayerAttack,
            EventType::BeforePlayerDeath,
            EventType::AfterPlayerDeath,
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
