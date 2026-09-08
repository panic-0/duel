//! 关联变化的校验、写入及提交后通知。

use super::{dispatch::Notice, World};
use crate::core::{
    buff_data::DestructionReason,
    event::Event,
    log::LogEntry,
    operation::{ChangeSet, DestroyedInfo, HpChange, HpRequest, OperationError, SubmissionResult},
    BuffId, PlayerId,
};
use std::any::Any;

struct StagedDestruction {
    buff_id: BuffId,
    owner: Option<PlayerId>,
    reason: DestructionReason,
    data: Box<dyn Any>,
}

impl World {
    // —— 受控提交与通知 ——

    /// 中性关联提交：全部关联变化写入完成后，才按
    /// “基础状态事实（提交声明顺序）→ 销毁事实（BuffId 升序）”开放通知；
    /// 每条事实的嵌套反应完整结束后，再继续本组下一条事实。
    pub(crate) fn submit(
        &mut self,
        changes: ChangeSet,
    ) -> Result<SubmissionResult, OperationError> {
        self.check_operation_failed()?;
        let ChangeSet {
            hp,
            remove_player,
            destroy,
            updates,
            hp_conflict,
            remove_player_conflict,
        } = changes;
        // 重复声明属于调用方错误：写入前拒绝，不静默丢弃请求。
        if hp_conflict {
            return Err(OperationError::Invalid(
                "ChangeSet 只允许声明一项生命变化".into(),
            ));
        }
        if remove_player_conflict {
            return Err(OperationError::Invalid(
                "ChangeSet 只允许声明一名角色移除".into(),
            ));
        }
        // 数据更新的契约边界：只接受与现有记录相同的实际类型；
        // 跨类型转换应表达为显式的移除／新增，而非普通的字段更新。
        // 不存在的实例在应用阶段跳过。
        // 注意：必须先解引用 Box 再取 TypeId，否则比较到的是 Box 自身类型。
        for (id, data) in &updates {
            if let Some(record) = self.records.get(id) {
                let existing: &dyn Any = record.data.as_ref();
                let incoming: &dyn Any = data.as_ref();
                if existing.type_id() != incoming.type_id() {
                    return Err(OperationError::Invalid(format!(
                        "update_data 的替换类型与实例 {id} 的现有类型不一致"
                    )));
                }
            }
        }
        let mut result = SubmissionResult::default();
        let mut staged: Vec<StagedDestruction> = Vec::new();

        // —— 关联写入阶段：不运行任何玩法响应 ——
        if let Some(request) = hp {
            result.hp = match request {
                HpRequest::Damage(target_id, amount) => self.commit_damage(target_id, amount),
                HpRequest::Heal(target_id, amount) => self.commit_heal(target_id, amount),
            };
        }
        if let Some(player_id) = remove_player {
            result.player_removed = self.take_player(player_id).is_some();
            if result.player_removed {
                let owned: Vec<BuffId> = self
                    .records
                    .iter()
                    .filter(|(_, record)| record.owner == Some(player_id))
                    .map(|(id, _)| *id)
                    .collect();
                for id in owned {
                    if let Some(record) = self.records.remove(&id) {
                        staged.push(StagedDestruction {
                            buff_id: id,
                            owner: record.owner,
                            reason: DestructionReason::OwnerDeath,
                            data: record.data,
                        });
                    }
                }
            }
        }
        for (id, reason) in destroy {
            // 同一实例只产生一次实际销毁事实。
            if staged.iter().any(|item| item.buff_id == id) {
                continue;
            }
            if let Some(record) = self.records.remove(&id) {
                staged.push(StagedDestruction {
                    buff_id: id,
                    owner: record.owner,
                    reason,
                    data: record.data,
                });
            }
        }
        // 数据更新按声明顺序应用，保留原实例身份；不存在的实例跳过。
        for (id, data) in updates {
            if let Some(record) = self.records.get_mut(&id) {
                record.data = data;
                result.updated.push(id);
            }
        }
        staged.sort_by_key(|item| item.buff_id);
        result.destroyed = staged
            .iter()
            .map(|item| DestroyedInfo {
                buff_id: item.buff_id,
                owner: item.owner,
                reason: item.reason,
            })
            .collect();

        // —— 通知阶段：基础状态事实 → 销毁事实 ——
        if let Some(change) = result.hp {
            if change.old_hp != change.new_hp {
                self.settle_and_record(Event::HpChanged {
                    target_id: change.target_id,
                    old_hp: change.old_hp,
                    new_hp: change.new_hp,
                })?;
            }
        }
        for item in staged {
            self.dispatch_notice(Notice::Destroyed {
                buff_id: item.buff_id,
                owner: item.owner,
                reason: item.reason,
                data: item.data,
            })?;
        }
        Ok(result)
    }

    // —— 中性生命变化（提交内部使用） ——

    fn commit_damage(&mut self, target_id: PlayerId, amount: u64) -> Option<HpChange> {
        let (old_hp, new_hp, max_hp) = {
            let target = self.state.get_player_mut(target_id)?;
            let old_hp = target.hp();
            let new_hp = target.damage(amount);
            (old_hp, new_hp, target.max_hp())
        };
        self.log(LogEntry::Damage {
            target_id,
            amount,
            hp: new_hp,
            max_hp,
        });
        Some(HpChange {
            target_id,
            old_hp,
            new_hp,
            max_hp,
            requested: if amount > i64::MAX as u64 {
                i64::MIN
            } else {
                -(amount as i64)
            },
            submitted_amount: amount,
        })
    }

    fn commit_heal(&mut self, target_id: PlayerId, amount: u64) -> Option<HpChange> {
        let (old_hp, new_hp, max_hp) = {
            let target = self.state.get_player_mut(target_id)?;
            let old_hp = target.hp();
            let new_hp = target.heal(amount);
            (old_hp, new_hp, target.max_hp())
        };
        self.log(LogEntry::Heal {
            target_id,
            amount,
            hp: new_hp,
            max_hp,
        });
        Some(HpChange {
            target_id,
            old_hp,
            new_hp,
            max_hp,
            requested: amount.min(i64::MAX as u64) as i64,
            submitted_amount: amount,
        })
    }
}
