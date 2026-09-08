//! 关联提交的声明与结果数据。

use crate::core::{buff_data::DestructionReason, BuffId, PlayerId};
use std::any::Any;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HpChange {
    pub target_id: PlayerId,
    pub old_hp: u64,
    pub new_hp: u64,
    pub max_hp: u64,
    pub requested: i64,
    /// 本次提交的无符号数值；伤害和治疗由调用的提交接口区分。
    pub submitted_amount: u64,
}

/// 一次关联提交的候选变化。全部变化在同一个提交边界内写入，
/// 期间不运行玩法响应；随后按“基础状态事实 → 销毁事实”顺序通知。
#[derive(Debug, Default)]
pub struct ChangeSet {
    pub(crate) hp: Option<HpRequest>,
    /// 是否重复声明了生命变化；提交时明确拒绝，不静默覆盖。
    pub(crate) hp_conflict: bool,
    pub(crate) remove_player: Option<PlayerId>,
    /// 是否重复声明了角色移除；提交时明确拒绝，不静默覆盖。
    pub(crate) remove_player_conflict: bool,
    pub(crate) destroy: Vec<(BuffId, DestructionReason)>,
    pub(crate) updates: Vec<(BuffId, Box<dyn Any>)>,
}

/// 无符号的生命变化请求；伤害与治疗分别走有界无符号计算。
#[derive(Debug, Clone, Copy)]
pub(crate) enum HpRequest {
    Damage(PlayerId, u64),
    Heal(PlayerId, u64),
}

impl ChangeSet {
    pub fn new() -> Self {
        Self::default()
    }

    fn set_hp_request(&mut self, request: HpRequest) {
        // 一次提交至多一项生命变化：重复声明在提交时明确拒绝，不静默覆盖。
        if self.hp.is_some() {
            self.hp_conflict = true;
        }
        self.hp = Some(request);
    }

    /// 生命变化（正数治疗、负数伤害）。
    pub fn hp(mut self, target_id: PlayerId, modifier: i64) -> Self {
        self.set_hp_request(if modifier < 0 {
            HpRequest::Damage(target_id, modifier.unsigned_abs())
        } else {
            HpRequest::Heal(target_id, modifier as u64)
        });
        self
    }

    /// 直接以无符号数值声明伤害，避免大数符号转换。
    pub fn damage(mut self, target_id: PlayerId, amount: u64) -> Self {
        self.set_hp_request(HpRequest::Damage(target_id, amount));
        self
    }

    /// 直接以无符号数值声明治疗。
    pub fn heal(mut self, target_id: PlayerId, amount: u64) -> Self {
        self.set_hp_request(HpRequest::Heal(target_id, amount));
        self
    }

    /// 移除一名角色；其 owner 依赖的数据实例随同次提交销毁（原因 OwnerDeath）。
    /// 与生命声明一致，重复移除声明在提交时明确拒绝，不静默覆盖。
    pub fn remove_player(mut self, id: PlayerId) -> Self {
        if self.remove_player.is_some() {
            self.remove_player_conflict = true;
        }
        self.remove_player = Some(id);
        self
    }

    /// 销毁一份数据实例并产生销毁事实。
    pub fn destroy(mut self, id: BuffId, reason: DestructionReason) -> Self {
        self.destroy.push((id, reason));
        self
    }

    /// 就地更新一份数据实例：保留原身份与候选顺序，不产生销毁事实。
    /// 多次更新同一实例时按声明顺序应用（最后一次生效）。
    pub fn update_data(mut self, id: BuffId, data: Box<dyn Any>) -> Self {
        self.updates.push((id, data));
        self
    }
}

/// 一份被销毁实例的元信息。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DestroyedInfo {
    pub buff_id: BuffId,
    pub owner: Option<PlayerId>,
    pub reason: DestructionReason,
}

#[derive(Debug, Default)]
pub struct SubmissionResult {
    /// 生命变化结果；未声明或目标不存在时为 `None`。
    pub hp: Option<HpChange>,
    /// 声明的角色移除是否实际发生。
    pub player_removed: bool,
    /// 本组提交销毁的全部实例，按 BuffId 升序。
    pub destroyed: Vec<DestroyedInfo>,
    /// 本组提交就地更新的实例身份，按声明顺序。
    pub updated: Vec<BuffId>,
}
