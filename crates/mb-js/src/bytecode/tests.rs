use super::*;

fn finish(emitter: Emitter) -> Code {
    emitter.finish(Vec::new(), 0, false)
}

fn decode_all(code: &Code) -> Vec<Instruction> {
    let mut instructions = Vec::new();
    let mut offset = 0;
    while offset < code.bytes().len() {
        let instruction = code.decode(offset).expect("gültiger Befehl");
        offset = instruction.end;
        instructions.push(instruction);
    }
    instructions
}

#[test]
fn every_opcode_has_its_own_byte() {
    assert!(Op::ALL.len() < 256);
    for (index, &op) in Op::ALL.iter().enumerate() {
        assert_eq!(usize::from(op.byte()), index);
        assert_eq!(Op::from_byte(op.byte()), Some(op));
    }
    assert_eq!(Op::from_byte(u8::MAX), None);
}

#[test]
fn operands_get_the_smallest_width() {
    let mut emitter = Emitter::default();
    emitter.emit(Op::Ldar, &[3]).expect("passt");
    emitter.emit(Op::Ldar, &[300]).expect("passt");
    emitter.emit(Op::Ldar, &[70_000]).expect("passt");
    emitter.emit(Op::LdaSmi, &[-128]).expect("passt");
    emitter.emit(Op::LdaSmi, &[128]).expect("passt");
    emitter.emit(Op::LdaSmi, &[-40_000]).expect("passt");
    // Ein breiter Operand macht alle breit.
    emitter.emit(Op::Mov, &[1, 256]).expect("passt");
    emitter.emit(Op::Return, &[]).expect("passt");
    let code = finish(emitter);
    let wide = Op::Wide.byte();
    let extra = Op::ExtraWide.byte();
    let expected = [
        vec![Op::Ldar.byte(), 3],
        vec![wide, Op::Ldar.byte(), 0x2c, 0x01],
        vec![extra, Op::Ldar.byte(), 0x70, 0x11, 0x01, 0x00],
        vec![Op::LdaSmi.byte(), 0x80],
        vec![wide, Op::LdaSmi.byte(), 0x80, 0x00],
        vec![extra, Op::LdaSmi.byte(), 0xc0, 0x63, 0xff, 0xff],
        vec![wide, Op::Mov.byte(), 1, 0, 0, 1],
        vec![Op::Return.byte()],
    ]
    .concat();
    assert_eq!(code.bytes(), expected.as_slice());

    let decoded: Vec<(Op, [i64; 2], usize, usize)> = decode_all(&code)
        .into_iter()
        .map(|instruction| {
            (
                instruction.op,
                instruction.operands,
                instruction.start,
                instruction.end,
            )
        })
        .collect();
    assert_eq!(
        decoded,
        [
            (Op::Ldar, [3, 0], 0, 2),
            (Op::Ldar, [300, 0], 2, 6),
            (Op::Ldar, [70_000, 0], 6, 12),
            (Op::LdaSmi, [-128, 0], 12, 14),
            (Op::LdaSmi, [128, 0], 14, 18),
            (Op::LdaSmi, [-40_000, 0], 18, 24),
            (Op::Mov, [1, 256], 24, 30),
            (Op::Return, [0, 0], 30, 31),
        ]
    );
}

#[test]
fn operands_outside_four_bytes_are_rejected() {
    let mut emitter = Emitter::default();
    assert_eq!(emitter.emit(Op::Ldar, &[-1]), Err(TooLarge));
    assert_eq!(emitter.emit(Op::Ldar, &[1 << 32]), Err(TooLarge));
    assert_eq!(emitter.emit(Op::LdaSmi, &[1 << 31]), Err(TooLarge));
    assert_eq!(emitter.emit(Op::Mov, &[1]), Err(TooLarge));
    assert!(emitter.emit(Op::Ldar, &[u32::MAX.into()]).is_ok());
    assert!(emitter.emit(Op::LdaSmi, &[i32::MIN.into()]).is_ok());
    assert_eq!(emitter.offset(), 12);
}

#[test]
fn broken_bytecode_does_not_decode() {
    let wide = Op::Wide.byte();
    let cases: [&[u8]; 4] = [
        // Ein unbekannter Opcode.
        &[u8::MAX],
        // Zwei Präfixe hintereinander.
        &[wide, wide, Op::Ldar.byte(), 0, 0],
        // Ein Operand fehlt.
        &[Op::Mov.byte(), 1],
        // Ein Präfix am Ende.
        &[wide],
    ];
    for bytes in cases {
        let code = Code {
            bytes: bytes.to_vec(),
            ..Code::default()
        };
        assert_eq!(code.decode(0), None, "{bytes:?}");
    }
    assert_eq!(Code::default().decode(0), None);
}

#[test]
fn forward_jumps_are_patched_and_loops_jump_back() {
    let mut emitter = Emitter::default();
    emitter.emit(Op::LdaTrue, &[]).expect("passt");
    let start = emitter.offset();
    let exit = emitter.jump(Op::JumpIfFalse).expect("passt");
    emitter.emit(Op::LdaFalse, &[]).expect("passt");
    emitter.jump_loop(start).expect("passt");
    emitter.bind(exit).expect("passt");
    emitter.emit(Op::Return, &[]).expect("passt");
    let code = finish(emitter);
    let instructions = decode_all(&code);
    let ops: Vec<Op> = instructions
        .iter()
        .map(|instruction| instruction.op)
        .collect();
    assert_eq!(
        ops,
        [
            Op::LdaTrue,
            Op::JumpIfFalse,
            Op::LdaFalse,
            Op::JumpLoop,
            Op::Return
        ]
    );
    let exit = instructions.get(1).expect("Sprung");
    let back = instructions.get(3).expect("Schleife");
    let end = instructions.get(4).expect("Ende");
    // Sprünge nach vorn haben immer vier Bytes.
    assert_eq!(exit.end - exit.start, 6);
    assert_eq!(exit.target(), Some(end.start));
    assert_eq!(back.target(), Some(start));
    assert!(back.operand(0) < 0);
}

#[test]
fn a_wide_loop_jump_counts_from_its_prefix() {
    let mut emitter = Emitter::default();
    for _ in 0..100 {
        emitter.emit(Op::LdaUndefined, &[]).expect("passt");
    }
    let before = emitter.offset();
    emitter.jump_loop(0).expect("passt");
    let code = finish(emitter);
    let back = code.decode(before).expect("Schleife");
    assert_eq!(back.op, Op::JumpLoop);
    assert_eq!(back.start, before);
    assert_eq!(back.target(), Some(0));

    let mut emitter = Emitter::default();
    for _ in 0..200 {
        emitter.emit(Op::LdaUndefined, &[]).expect("passt");
    }
    emitter.jump_loop(0).expect("passt");
    let code = finish(emitter);
    let back = code.decode(200).expect("Schleife");
    assert_eq!(code.bytes().get(200), Some(&Op::Wide.byte()));
    assert_eq!(back.target(), Some(0));
}

#[test]
fn positions_follow_the_instructions() {
    let mut emitter = Emitter::default();
    emitter.set_position(5).expect("passt");
    emitter.emit(Op::LdaNull, &[]).expect("passt");
    emitter.set_position(5).expect("passt");
    emitter.emit(Op::LdaNull, &[]).expect("passt");
    emitter.set_position(9).expect("passt");
    // Ohne Befehl dazwischen gilt die letzte Lage.
    emitter.set_position(12).expect("passt");
    emitter.emit(Op::Return, &[]).expect("passt");
    let code = finish(emitter);
    assert_eq!(code.position(0), Some(5));
    assert_eq!(code.position(1), Some(5));
    assert_eq!(code.position(2), Some(12));
    assert_eq!(code.position(100), Some(12));
    assert_eq!(Code::default().position(0), None);
}

#[test]
fn disassembly_names_registers_constants_and_targets() {
    let mut emitter = Emitter::default();
    emitter.emit(Op::LdaConstant, &[0]).expect("passt");
    emitter.emit(Op::Star, &[2]).expect("passt");
    emitter.emit(Op::LdaConstant, &[1]).expect("passt");
    emitter.emit(Op::Add, &[2]).expect("passt");
    let skip = emitter.jump(Op::JumpIfTrue).expect("passt");
    emitter.emit(Op::LdaGlobal, &[2]).expect("passt");
    emitter.bind(skip).expect("passt");
    emitter.emit(Op::LdaSmi, &[-7]).expect("passt");
    emitter.emit(Op::LdaConstant, &[9]).expect("passt");
    emitter.emit(Op::Return, &[]).expect("passt");
    let constants = vec![
        Constant::Number(0.5),
        Constant::String(vec![u16::from(b'a'), 0xd800, u16::from(b'"')]),
        Constant::Name("x".encode_utf16().collect()),
    ];
    let code = emitter.finish(constants, 3, true);
    assert!(code.is_strict());
    assert_eq!(code.register_count(), 3);
    assert_eq!(
        code.to_string(),
        "   0: LdaConstant 0.5\n\
         \x20  2: Star r2\n\
         \x20  4: LdaConstant \"a\\u{d800}\\\"\"\n\
         \x20  6: Add r2\n\
         \x20  8: JumpIfTrue @16\n\
         \x20 14: LdaGlobal x\n\
         \x20 16: LdaSmi -7\n\
         \x20 18: LdaConstant c9?\n\
         \x20 20: Return\n"
    );
}
