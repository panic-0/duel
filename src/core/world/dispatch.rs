//! 事实通知的候选快照、统一排序与同步响应。

use super::World;
use crate::core::{
    buff_data::DestructionReason,
    event::Event,
    operation::OperationError,
    system::{Fact, NoticeKind, Priority, Subject},
    BuffId, PlayerId,
};
use std::any::Any;

#[derive(Debug, Clone, Copy)]
struct RankedCandidate {
    priority: Priority,
    subject_order: usize,
    system_order: usize,
    system_id: usize,
    subject: Subject,
}

/// 一次分发的事实：普通事件或携带暂存数据的销毁事实。
pub(super) enum Notice {
    Event(Event),
    Destroyed {
        buff_id: BuffId,
        owner: Option<PlayerId>,
        reason: DestructionReason,
        data: Box<dyn Any>,
    },
}

impl Notice {
    fn kind(&self) -> NoticeKind {
        match self {
            Notice::Event(event) => NoticeKind::Event(event.event_type()),
            Notice::Destroyed { .. } => NoticeKind::Destroyed,
        }
    }

    fn fact(&self) -> Fact<'_> {
        match self {
            Notice::Event(event) => Fact::Event(event),
            Notice::Destroyed {
                buff_id,
                owner,
                reason,
                data,
            } => Fact::Destroyed(crate::core::system::Destruction {
                buff_id: *buff_id,
                owner: *owner,
                reason: *reason,
                data: data.as_ref(),
            }),
        }
    }
}

impl World {
    pub(crate) fn settle_operation_event(&mut self, event: Event) -> Result<(), OperationError> {
        // 发布通知是运行期受控入口：世界已记录失败时直接拒绝。
        self.check_operation_failed()?;
        self.dispatch_notice(Notice::Event(event))
    }

    /// 分发一条事实：开始时统一收集候选并按
    /// `(Priority, 响应主体稳定顺序, System 注册顺序)` 升序固定，
    /// 然后逐候选响应；每个候选返回的 Operation 及其反应完整结束后再继续。
    pub(super) fn dispatch_notice(&mut self, notice: Notice) -> Result<(), OperationError> {
        if self.end {
            // 正式终局后不再启动后续玩法监听者。
            return Ok(());
        }
        let routes = self
            .notice_index
            .get(&notice.kind())
            .cloned()
            .unwrap_or_default();
        let mut candidates: Vec<RankedCandidate> = Vec::new();
        {
            let fact = notice.fact();
            let query = self.query();
            for route in &routes {
                let slot = &self.systems[route.system_id];
                for subject in slot.system.candidates(&fact, &query) {
                    let subject_order = match subject {
                        Subject::Instance(id) => query.subject_order(id).unwrap_or(0),
                        Subject::Standalone => route.system_order,
                    };
                    candidates.push(RankedCandidate {
                        priority: route.priority,
                        subject_order,
                        system_order: route.system_order,
                        system_id: route.system_id,
                        subject,
                    });
                }
            }
        }
        candidates.sort_by(|a, b| {
            (a.priority, a.subject_order, a.system_order).cmp(&(
                b.priority,
                b.subject_order,
                b.system_order,
            ))
        });

        for candidate in candidates {
            if self.is_end() {
                break;
            }
            // C3A：轮到候选时检查实例仍存在；新增实例不补收当前通知。
            if let Subject::Instance(id) = candidate.subject {
                if !self.records.contains_key(&id) {
                    continue;
                }
            }
            let outcome = {
                let fact = notice.fact();
                let query = self.query();
                self.systems[candidate.system_id]
                    .system
                    .respond(&fact, candidate.subject, &query)
            };
            let ops = match outcome {
                Ok(ops) => ops,
                Err(error) => {
                    self.record_operation_error(error.clone());
                    return Err(error);
                }
            };
            // D4：候选返回的 Operation 已产生，即使后续反应销毁了来源数据也继续执行。
            for operation in ops {
                if self.is_end() {
                    break;
                }
                self.execute_inline(operation)?;
            }
        }
        Ok(())
    }

    /// 结算事件；错误在记录后原样返回，保证失败状态对后续受控入口可见。
    pub(super) fn settle_and_record(&mut self, event: Event) -> Result<(), OperationError> {
        let result = self.dispatch_notice(Notice::Event(event));
        if let Err(error) = &result {
            self.record_operation_error(error.clone());
        }
        result
    }
}
