//! 技能持有者资格、稳定槽位与行动间检查点。

use crate::support::CountOperation;
use duel::core::{
    business::skills::{Abilities, Ability},
    event::{Checkpoint, Event, EventKind},
    install_default_rules,
    operation::{completed, ActionContext, Operation, OperationError, OperationOutcome},
    player::Player,
    query::Query,
    state::BattleState,
    system::{EventEnvelope, Priority, ReactionTarget, System},
    Attack, BattleEngine, Damage, DuelRunner, Heal, PlayerId, TurnOperation,
};
use std::{cell::Cell, rc::Rc};

/// 记录被使用次数的技能。
#[derive(Debug)]
struct TouchAbility(Rc<Cell<usize>>);

impl Ability for TouchAbility {
    fn operation(&self, _source: PlayerId, _state: &BattleState) -> Option<Box<dyn Operation>> {
        self.0.set(self.0.get() + 1);
        None
    }
}

#[test]
fn abilities_holder_field_drives_turn_matching() {
    let mut world = BattleEngine::new();
    let a = world.add_player(Player::new("A".into(), 10, 0));
    let b = world.add_player(Player::new("B".into(), 10, 0));
    let used = Rc::new(Cell::new(0));
    // owner=A（随 A 销毁），持有者=B：B 的回合应使用这份技能集合。
    world.attach_component(
        Some(a),
        Abilities::new(b, vec![Box::new(TouchAbility(used.clone()))]),
    );

    world
        .execute(TurnOperation {
            round: 1,
            player_id: b,
        })
        .expect("B 的回合应正常执行");

    assert_eq!(
        used.get(),
        1,
        "Turn 应按业务持有者字段匹配技能集合，而不是记录 owner"
    );
}

#[derive(Debug)]
struct CountingAbility(Rc<Cell<usize>>);

impl Ability for CountingAbility {
    fn operation(&self, _: PlayerId, _: &BattleState) -> Option<Box<dyn Operation>> {
        Some(Box::new(CountOperation(self.0.clone())))
    }
}

#[derive(Debug)]
struct EndOperation;

impl Operation for EndOperation {
    fn execute(
        self: Box<Self>,
        ctx: &mut ActionContext<'_>,
    ) -> Result<OperationOutcome, OperationError> {
        ctx.end_game(duel::core::BattleResult::Draw)?;
        completed()
    }
}

#[derive(Debug)]
struct EndAtFirstActionCheckpoint;

impl System for EndAtFirstActionCheckpoint {
    fn subscriptions(&self) -> Vec<(EventKind, Priority)> {
        vec![(EventKind::Checkpoint, Priority::Default)]
    }
    fn candidates(
        &self,
        fact: &EventEnvelope<'_>,
        _query: &duel::core::query::Query<'_>,
    ) -> Vec<ReactionTarget> {
        matches!(
            fact.event(),
            Some(Event::Checkpoint {
                phase: Checkpoint::ActionEnd,
                ..
            })
        )
        .then(|| vec![ReactionTarget::Standalone])
        .unwrap_or_default()
    }
    fn respond(
        &self,
        _fact: &EventEnvelope<'_>,
        _subject: ReactionTarget,
        _query: &duel::core::query::Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        Ok(vec![Box::new(EndOperation)])
    }
}

#[test]
fn checkpoint_between_normal_abilities_stops_second_ability() {
    let mut world = BattleEngine::new();
    let a = world.add_player(Player::new("A".into(), 10, 0));
    world.add_player(Player::new("B".into(), 10, 0));
    let count = Rc::new(Cell::new(0));
    world.attach_component(
        Some(a),
        Abilities::new(
            a,
            vec![
                Box::new(CountingAbility(count.clone())),
                Box::new(CountingAbility(count.clone())),
            ],
        ),
    );
    world.register_system(EndAtFirstActionCheckpoint);
    world.run_with_max_rounds(1).expect("对局应正常结束");
    assert_eq!(
        count.get(),
        1,
        "两个正常技能都在第一个 ActionEnd 检查点之前执行"
    );
}

/// 自毁：对使用者自己造成 20 点伤害（20 血角色会死亡）。
#[derive(Debug)]
struct SelfDestruct;

impl Ability for SelfDestruct {
    fn operation(&self, actor: PlayerId, _state: &BattleState) -> Option<Box<dyn Operation>> {
        Some(Box::new(Damage::new(None, actor, 20)))
    }
}

/// 治疗队友 3 点。
#[derive(Debug)]
struct HealAlly(PlayerId);

impl Ability for HealAlly {
    fn operation(&self, _actor: PlayerId, _state: &BattleState) -> Option<Box<dyn Operation>> {
        Some(Box::new(Heal::new(self.0, 3)))
    }
}

#[test]
fn dead_holder_does_not_get_a_new_normal_action_from_surviving_skill_component() {
    let mut world = BattleEngine::new();
    install_default_rules(&mut world);
    let sponsor = world.add_player(Player::new("Sponsor".into(), 20, 0));
    let actor = world.add_player(Player::new("Actor".into(), 20, 0));
    let ally = world.add_player(Player::new("Ally".into(), 20, 0));
    assert!(world.set_initial_hp(ally, 10)); // 仅用于测试初始化。

    // 生命周期依赖是 Sponsor；技能的使用者是 Actor。
    let skills = world.attach_component(
        Some(sponsor),
        Abilities::new(
            actor,
            vec![Box::new(SelfDestruct), Box::new(HealAlly(ally))],
        ),
    );

    world
        .execute(TurnOperation {
            round: 1,
            player_id: actor,
        })
        .expect("死亡链与回合收尾应正常结算");

    assert!(world.player(actor).is_none(), "自毁应使持有者死亡");
    assert!(
        world
            .component::<duel::core::business::skills::Abilities>(skills)
            .is_some(),
        "owner 仍存活，技能数据应保留"
    );
    assert!(
        !world.is_end(),
        "Sponsor 与 Ally 仍在，终局判断不得掩盖行动资格问题"
    );
    assert_eq!(
        world.player(ally).unwrap().hp(),
        10,
        "持有者死亡后不得再安排新的正常技能：数据存活不等于资格成立"
    );
}

/// 在 Turn 通知的响应中杀死行动者的 System。
#[derive(Debug)]
struct KillOnTurn(PlayerId);

impl System for KillOnTurn {
    fn subscriptions(&self) -> Vec<(EventKind, Priority)> {
        vec![(EventKind::Turn, Priority::Default)]
    }

    fn candidates(&self, fact: &EventEnvelope<'_>, _query: &Query<'_>) -> Vec<ReactionTarget> {
        match fact.event() {
            Some(Event::Turn { player_id, .. }) if *player_id == self.0 => {
                vec![ReactionTarget::Standalone]
            }
            _ => Vec::new(),
        }
    }

    fn respond(
        &self,
        _fact: &EventEnvelope<'_>,
        _subject: ReactionTarget,
        _query: &Query<'_>,
    ) -> Result<Vec<Box<dyn Operation>>, OperationError> {
        Ok(vec![Box::new(Damage::new(None, self.0, 20))])
    }
}

#[test]
fn holder_killed_by_turn_event_does_not_get_a_first_normal_action() {
    let mut world = BattleEngine::new();
    install_default_rules(&mut world);
    let sponsor = world.add_player(Player::new("Sponsor".into(), 20, 0));
    let actor = world.add_player(Player::new("Actor".into(), 20, 0));
    let ally = world.add_player(Player::new("Ally".into(), 20, 0));
    assert!(world.set_initial_hp(ally, 10));
    world.attach_component(
        Some(sponsor),
        Abilities::new(actor, vec![Box::new(HealAlly(ally))]),
    );
    world.register_system(KillOnTurn(actor));

    world
        .execute(TurnOperation {
            round: 1,
            player_id: actor,
        })
        .expect("回合应在行动者死亡后正常收尾");

    assert!(world.player(actor).is_none());
    assert_eq!(
        world.player(ally).unwrap().hp(),
        10,
        "Turn 通知前的存活检查在反应后已过期，不得据此安排第一项正常技能"
    );
}

/// 未满血时治疗的技能；治疗完成后即不可用。
#[derive(Debug)]
struct HealIfWounded;

impl Ability for HealIfWounded {
    fn operation(&self, source: PlayerId, state: &BattleState) -> Option<Box<dyn Operation>> {
        let player = state.player(source)?;
        if player.hp() < player.max_hp() {
            Some(Box::new(Heal::new(source, player.max_hp() - player.hp())))
        } else {
            None
        }
    }
}

#[test]
fn normal_action_cursor_does_not_skip_attack_after_conditional_heal_disappears() {
    let mut world = BattleEngine::new();
    let a = world.add_player(Player::new("A".into(), 10, 3));
    let b = world.add_player(Player::new("B".into(), 20, 0));
    assert!(world.set_initial_hp(a, 9)); // 仅用于测试初始化。
    world.attach_component(
        Some(a),
        Abilities::new(a, vec![Box::new(HealIfWounded), Box::new(Attack)]),
    );

    world
        .execute(TurnOperation {
            round: 1,
            player_id: a,
        })
        .expect("回合应正常执行");

    assert_eq!(world.player(a).unwrap().hp(), 10);
    assert_eq!(
        world.player(b).unwrap().hp(),
        17,
        "治疗后不可用的技能不应让普攻因过滤列表索引左移而被跳过"
    );
}
