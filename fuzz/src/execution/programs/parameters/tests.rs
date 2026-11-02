use super::super::{ErrorKind, Outcome, Primitive, exercise, program, run};

fn numbers(values: &[f64]) -> Vec<Outcome> {
    values
        .iter()
        .map(|&value| Outcome::Value(Primitive::number(value)))
        .collect()
}

#[test]
fn captured_cells_are_shared_by_siblings_and_separate_between_factories() {
    for (family, values) in [
        (32, vec![14.0, 17.0, 22.0]),
        (33, vec![14.0, 14.0, 17.0, 17.0]),
        (34, vec![8.0, 12.0, 17.0, 17.0]),
        (35, vec![14_012.0, 17_015.0]),
    ] {
        for stress in [false, true] {
            assert_eq!(run(&program(&[family, 5, 9, 3]), stress), numbers(&values));
        }
    }
}

#[test]
fn every_loop_depth_distinguishes_var_let_and_block_const_captures() {
    for count in 0..=8 {
        for family in 37..=39 {
            exercise(&[family, 250, 7, count]);
        }
    }
    for (family, expected) in [(37, -20.0), (38, 16.0), (39, -20.0)] {
        assert_eq!(
            run(&program(&[family, 250, 7, 8]), true),
            numbers(&[expected])
        );
    }
}

#[test]
fn mapping_depends_on_strictness_duplicates_presence_deletion_and_formals() {
    for (family, values) in [
        (44, vec![9.0, 3.0, 12.0]),
        (45, vec![5.0, 5.0]),
        (46, vec![3_005.0, 5_003.0, 5.0]),
        (47, vec![9_001.0, 3_002.0, 3_003.0]),
        (49, vec![5.0, 9.0, 3.0]),
        (54, vec![5_003.0, 3.0, 3.0]),
        (59, vec![5.0, 5.0, 0.0]),
    ] {
        assert_eq!(run(&program(&[family, 5, 9, 3]), true), numbers(&values));
    }
    assert_eq!(
        run(&program(&[48, 5, 9, 3]), true),
        [
            Outcome::Value(Primitive::number(9_003.0)),
            Outcome::Value(Primitive::number(3.0)),
            Outcome::Value(Primitive::Boolean(false)),
        ]
    );
}

#[test]
fn defaults_run_in_order_and_keep_their_parameter_environment() {
    for (family, values) in [
        (42, vec![10.0, 19.0, 38.0]),
        (50, vec![19.0, 12.0, 15.0, 122.0]),
        (52, vec![5.0, 5.0, 3.0]),
        (58, vec![5.0, 9.0, 3.0, 9.0]),
    ] {
        assert_eq!(run(&program(&[family, 5, 9, 3]), true), numbers(&values));
    }
    assert_eq!(
        run(&program(&[51, 5, 9, 3]), true),
        [
            Outcome::Value(Primitive::number(5.0)),
            Outcome::Value(Primitive::number(5.0)),
            Outcome::Value(Primitive::Null),
            Outcome::Value(Primitive::Boolean(true)),
        ]
    );
}

#[test]
fn escaped_tdz_and_failed_initializers_preserve_state_and_allow_following_calls() {
    assert_eq!(
        run(&program(&[36, 5, 9, 3]), true),
        [
            Outcome::Error(ErrorKind::ReferenceError),
            Outcome::Error(ErrorKind::ReferenceError),
            Outcome::Value(Primitive::number(12.0)),
        ]
    );
    assert_eq!(
        run(&program(&[53, 5, 9, 3]), true),
        [
            Outcome::Error(ErrorKind::ReferenceError),
            Outcome::Value(Primitive::number(1.0)),
            Outcome::Value(Primitive::number(9.0)),
            Outcome::Value(Primitive::number(1001.0)),
        ]
    );
    assert_eq!(
        run(&program(&[63, 5, 9, 3]), true),
        [
            Outcome::Thrown(Primitive::number(14.0)),
            Outcome::Value(Primitive::number(14.0)),
            Outcome::Thrown(Primitive::number(17.0)),
            Outcome::Value(Primitive::number(17.0)),
        ]
    );
}

#[test]
fn default_and_rest_captures_keep_lone_surrogates_alive_across_gc() {
    for (bytes, expected) in [
        ([62, 0xd8, 0, 0], vec![0xd800, 0]),
        ([62, 0xdc, 0, 2], vec![0xdc00, 2]),
        ([62, 0, 0, 1], vec![0, 0xdc01]),
    ] {
        let expected = Outcome::Value(Primitive::String(expected));
        assert_eq!(run(&program(&bytes), true), vec![expected; 3]);
    }
}
