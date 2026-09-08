//! 根操作、子操作工作栈及失败状态传播。

use super::BattleEngine;
use crate::core::operation::{
    ActionContext, ErasedOperation, Operation, OperationError, OperationOutcome, OperationResult,
};
use std::panic::{catch_unwind, AssertUnwindSafe};

/// 反应链的最大同步嵌套深度。子操作队列是迭代的，不受此限制；
/// 只有“操作发布通知 → System 响应 → 再发布通知”的递归路径受它约束。
const MAX_REACTION_DEPTH: usize = 256;

impl BattleEngine {
    // —— 根入口与执行 ——

    /// 运行一个根 Operation。正式终局后的新根请求被无副作用拒绝，
    /// 不把正常终局记为执行失败；不自动装配默认规则。
    pub fn execute<O: Operation>(
        &mut self,
        operation: O,
    ) -> Result<OperationOutcome, OperationError> {
        if self.end {
            return Err(OperationError::Invalid("对局已结束".into()));
        }
        self.check_operation_failed()?;
        let result = self.execute_erased(Box::new(operation));
        if let Err(error) = &result {
            self.record_operation_error(error.clone());
        }
        result
    }

    fn execute_erased(
        &mut self,
        operation: Box<dyn ErasedOperation>,
    ) -> Result<OperationOutcome, OperationError> {
        // 进入任何操作前先拦截已有失败与终局；终局属正常状态，按跳过处理。
        self.check_operation_failed()?;
        if self.end {
            return Ok((OperationResult::Skipped, None));
        }
        if self.operation_depth >= MAX_REACTION_DEPTH {
            return Err(OperationError::Failed(format!(
                "反应链嵌套深度超过上限 {MAX_REACTION_DEPTH}"
            )));
        }
        self.operation_depth += 1;
        let outcome = (|| {
            let mut stack: Vec<Box<dyn ErasedOperation>> = vec![operation];
            let mut root_result = None;
            while let Some(op) = stack.pop() {
                // 正式终局后不再启动已排队但尚未开始的玩法操作；
                // 必要的资源释放与运行时收尾由各操作自身保证。
                if self.end {
                    break;
                }
                let result = {
                    let mut context = ActionContext::new(self);
                    let result = catch_unwind(AssertUnwindSafe(|| op.execute_erased(&mut context)))
                        .map_err(|_| {
                            OperationError::Failed("Operation panic，BattleEngine 已停止".into())
                        })
                        .and_then(|value| value);
                    let children = context.take_children();
                    drop(context);
                    for child in children.into_iter().rev() {
                        stack.push(child);
                    }
                    result
                };
                let value = result?;
                if let Some(error) = &self.operation_error {
                    return Err(error.clone());
                }
                if root_result.is_none() {
                    root_result = Some(value);
                }
            }
            Ok(root_result.expect("Operation 栈不应为空"))
        })();
        self.operation_depth -= 1;
        outcome
    }

    pub(crate) fn execute_child(
        &mut self,
        operation: Box<dyn ErasedOperation>,
    ) -> Result<OperationOutcome, OperationError> {
        let result = self.execute_erased(operation);
        if let Err(error) = &result {
            self.record_operation_error(error.clone());
        }
        result
    }

    pub(super) fn execute_inline(
        &mut self,
        operation: Box<dyn Operation>,
    ) -> Result<OperationOutcome, OperationError> {
        let result = self.execute_erased(Box::new(operation));
        if let Err(error) = &result {
            self.record_operation_error(error.clone());
        }
        result
    }

    /// 引擎已记录失败时返回该错误；受控入口在执行任何新变化前检查。
    pub(crate) fn check_operation_failed(&self) -> Result<(), OperationError> {
        match &self.operation_error {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }

    pub(crate) fn record_operation_error(&mut self, error: OperationError) {
        if self.operation_error.is_none() {
            self.operation_error = Some(error);
        }
    }
}
