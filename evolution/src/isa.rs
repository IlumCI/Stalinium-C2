//! The instruction set. Thirty-two ops are Ray's Tierra set (1991), one is new:
//! `sense`, which lets a creature feel where the sunlight is.

pub const NOP0: u8 = 0;
pub const NOP1: u8 = 1;
pub const OR1: u8 = 2;
pub const SHL: u8 = 3;
pub const ZERO: u8 = 4;
pub const IF_CZ: u8 = 5;
pub const SUB_AB: u8 = 6;
pub const SUB_AC: u8 = 7;
pub const INC_A: u8 = 8;
pub const INC_B: u8 = 9;
pub const DEC_C: u8 = 10;
pub const INC_C: u8 = 11;
pub const PUSH_AX: u8 = 12;
pub const PUSH_BX: u8 = 13;
pub const PUSH_CX: u8 = 14;
pub const PUSH_DX: u8 = 15;
pub const POP_AX: u8 = 16;
pub const POP_BX: u8 = 17;
pub const POP_CX: u8 = 18;
pub const POP_DX: u8 = 19;
pub const JMP: u8 = 20;
pub const JMPB: u8 = 21;
pub const CALL: u8 = 22;
pub const RET: u8 = 23;
pub const MOV_CD: u8 = 24;
pub const MOV_AB: u8 = 25;
pub const MOV_IAB: u8 = 26;
pub const ADR: u8 = 27;
pub const ADRB: u8 = 28;
pub const ADRF: u8 = 29;
pub const MAL: u8 = 30;
pub const DIVIDE: u8 = 31;
pub const SENSE: u8 = 32;
pub const NOPX: u8 = 33;
pub const NAND: u8 = 34;
pub const IO: u8 = 35;
pub const DIVE: u8 = 36;
pub const GUESS: u8 = 37;
pub const LOAD: u8 = 38;
pub const STORE: u8 = 39;
pub const INFECT: u8 = 40;
pub const EMIT: u8 = 41;
pub const TUNE: u8 = 42;
pub const ENCLOSE: u8 = 43;
pub const LEVY: u8 = 44;
pub const REVOLT: u8 = 45;
pub const GIVE: u8 = 46;
pub const MATE: u8 = 47;
pub const LEARN: u8 = 48;
pub const TEACH: u8 = 49;
pub const JOIN: u8 = 50;
pub const EAT: u8 = 51;

pub const NUM_OPS: u8 = 52;

pub const NAMES: [&str; NUM_OPS as usize] = [
    "nop0", "nop1", "or1", "shl", "zero", "if_cz", "sub_ab", "sub_ac", "inc_a", "inc_b", "dec_c",
    "inc_c", "push_ax", "push_bx", "push_cx", "push_dx", "pop_ax", "pop_bx", "pop_cx", "pop_dx",
    "jmp", "jmpb", "call", "ret", "mov_cd", "mov_ab", "mov_iab", "adr", "adrb", "adrf", "mal",
    "divide", "sense", "nopx", "nand", "io", "dive", "guess", "load", "store", "infect", "emit", "tune", "enclose", "levy", "revolt", "give", "mate", "learn", "teach", "join", "eat",
];

/// Avida's nine logic tasks (Lenski et al. 2003), cheapest first, then echo: output a
/// number you just read. Echo is the first stepping stone; it makes `io` worth keeping.
pub const TASKS: [&str; 10] = ["not", "nand", "and", "orn", "or", "andn", "nor", "xor", "equ", "echo"];
/// CPU-time multiplier for each task, by how many NANDs it takes to build.
pub const TASK_BONUS: [u32; 10] = [2, 2, 4, 4, 8, 8, 16, 16, 32, 2];

pub fn assemble(src: &str) -> Result<Vec<u8>, String> {
    src.lines()
        .flat_map(|l| l.split(';').next().unwrap_or("").split_whitespace())
        .map(|t| NAMES.iter().position(|n| *n == t).map(|i| i as u8).ok_or(format!("unknown op `{t}`")))
        .collect()
}

/// One instruction per line, with nop runs folded onto the op that owns them.
pub fn disassemble(code: &[u8]) -> String {
    let mut out = String::new();
    for (i, &op) in code.iter().enumerate() {
        if op <= NOP1 && i > 0 {
            out.push(' ');
        } else if i > 0 {
            out.push('\n');
        }
        out.push_str(NAMES[(op % NUM_OPS) as usize]);
    }
    out.push('\n');
    out
}

/// The seed replicator: Ray's 80-instruction ancestor, rebuilt for this VM, plus a
/// scratch region at the top of the reproduction loop: 40 cells of `nopx` that do nothing,
/// bracketed so whatever evolves there can't clobber the copy machinery. It's where
/// computation on the outside world gets a chance to appear (Avida's ancestor carries
/// similar filler).
///
/// `mov_cd` sets dx, which `mal` reads as "where to put my child". Swapping it for
/// `sense` is a single point mutation, and it sends children toward the sun.
pub const ANCESTOR: &str = "
    nop1 nop1 nop1 nop1        ; I start here
    zero or1 shl shl           ; cx = 4, the template length
    mov_cd                     ; dx = 4: put my child just past me
    adrb nop0 nop0 nop0 nop0   ; ax = just after my start template
    sub_ac                     ; ax = my first cell
    mov_ab                     ; bx = my first cell
    adrf nop0 nop0 nop0 nop1   ; ax = my last cell
    inc_a
    sub_ab                     ; cx = my size
    nop1 nop1 nop0 nop1        ; reproduction loop
    push_bx push_cx            ; scratch region: start and size are saved
    nopx nopx nopx nopx nopx nopx nopx nopx nopx nopx
    nopx nopx nopx nopx nopx nopx nopx nopx nopx nopx
    nopx nopx nopx nopx nopx nopx nopx nopx nopx nopx
    nopx nopx nopx nopx nopx nopx nopx nopx nopx nopx
    pop_cx pop_bx
    mal                        ; ax = room for a child
    call nop0 nop0 nop1 nop1   ; copy myself there
    divide                     ; let it go
    jmpb nop0 nop0 nop1 nop0   ; again (backward only, past the scratch region)
    if_cz
    nop1 nop1 nop0 nop0        ; copy procedure
    push_ax push_bx push_cx
    nop1 nop0 nop1 nop0        ; copy loop
    mov_iab
    dec_c
    if_cz
    jmp nop0 nop1 nop0 nop0    ; done: leave
    inc_a inc_b
    jmp nop0 nop1 nop0 nop1    ; next cell
    if_cz
    nop1 nop0 nop1 nop1        ; exit
    pop_cx pop_bx pop_ax
    ret
    nop1 nop1 nop1 nop0        ; I end here
    if_cz
";

/// Every task's output for inputs a, b. `not` only looks at a.
/// NOT in five instructions: io push_cx pop_dx nand io. nand is cx = !(cx & dx).
pub fn task_outputs(a: u32, b: u32) -> [u32; 9] {
    [!a, !(a & b), a & b, a | !b, a | b, a & !b, !(a | b), a ^ b, !(a ^ b)]
}