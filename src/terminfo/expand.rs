//! Bounded evaluator for the numeric terminfo expressions used by cursor capabilities.
use std::fmt;

const MAX_STACK: usize = 32;
const MAX_OUTPUT: usize = 4096;
const MAX_NESTING: usize = 16;

#[derive(Debug)]
pub(super) struct ExpandError(&'static str);

impl fmt::Display for ExpandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for ExpandError {}

#[derive(Clone, Copy)]
struct Branch {
    parent_active: bool,
    condition: Option<bool>,
    saw_else: bool,
}

fn push(stack: &mut Vec<i32>, value: i32) -> Result<(), ExpandError> {
    if stack.len() >= MAX_STACK {
        return Err(ExpandError("terminfo parameter stack overflow"));
    }
    stack.push(value);
    Ok(())
}
fn pop(stack: &mut Vec<i32>) -> Result<i32, ExpandError> {
    stack
        .pop()
        .ok_or(ExpandError("terminfo parameter stack underflow"))
}
fn append(out: &mut Vec<u8>, bytes: &[u8]) -> Result<(), ExpandError> {
    if bytes.len() > MAX_OUTPUT.saturating_sub(out.len()) {
        return Err(ExpandError("terminfo expansion too long"));
    }
    out.extend_from_slice(bytes);
    Ok(())
}

pub(super) fn expand(template: &[u8], mut params: [i32; 2]) -> Result<Vec<u8>, ExpandError> {
    if template.len() > MAX_OUTPUT {
        return Err(ExpandError("terminfo capability too long"));
    }
    let mut out = Vec::with_capacity(template.len());
    let mut stack = Vec::new();
    let mut branches: Vec<Branch> = Vec::new();
    let mut active = true;
    let mut at = 0;
    while at < template.len() {
        if template[at] != b'%' {
            if active {
                append(&mut out, &template[at..at + 1])?;
            }
            at += 1;
            continue;
        }
        at += 1;
        let op = *template
            .get(at)
            .ok_or(ExpandError("incomplete terminfo directive"))?;
        at += 1;
        match op {
            b'?' => {
                if branches.len() >= MAX_NESTING {
                    return Err(ExpandError("terminfo conditional nesting too deep"));
                }
                branches.push(Branch {
                    parent_active: active,
                    condition: None,
                    saw_else: false,
                });
            }
            b't' => {
                let branch = branches.last_mut().ok_or(ExpandError("unexpected %t"))?;
                if branch.condition.is_some() {
                    return Err(ExpandError("duplicate %t"));
                }
                let condition = if branch.parent_active {
                    pop(&mut stack)? != 0
                } else {
                    false
                };
                branch.condition = Some(condition);
                active = branch.parent_active && condition;
            }
            b'e' => {
                let branch = branches.last_mut().ok_or(ExpandError("unexpected %e"))?;
                let condition = branch.condition.ok_or(ExpandError("%e before %t"))?;
                if branch.saw_else {
                    return Err(ExpandError("duplicate %e"));
                }
                branch.saw_else = true;
                active = branch.parent_active && !condition;
            }
            b';' => {
                let branch = branches.pop().ok_or(ExpandError("unexpected %;"))?;
                if branch.condition.is_none() {
                    return Err(ExpandError("%; before %t"));
                }
                active = branch.parent_active;
            }
            b'%' => {
                if active {
                    append(&mut out, b"%")?;
                }
            }
            b'i' => {
                if active {
                    params[0] = params[0]
                        .checked_add(1)
                        .ok_or(ExpandError("parameter overflow"))?;
                    params[1] = params[1]
                        .checked_add(1)
                        .ok_or(ExpandError("parameter overflow"))?;
                }
            }
            b'p' => {
                let index = *template
                    .get(at)
                    .ok_or(ExpandError("missing parameter number"))?;
                at += 1;
                if !(b'1'..=b'2').contains(&index) {
                    return Err(ExpandError("unsupported parameter number"));
                }
                if active {
                    push(&mut stack, params[usize::from(index - b'1')])?;
                }
            }
            b'{' => {
                let end = template[at..]
                    .iter()
                    .position(|byte| *byte == b'}')
                    .ok_or(ExpandError("unterminated numeric constant"))?
                    + at;
                if end - at > 12 {
                    return Err(ExpandError("numeric constant too long"));
                }
                let value = std::str::from_utf8(&template[at..end])
                    .ok()
                    .and_then(|text| text.parse::<i32>().ok())
                    .ok_or(ExpandError("invalid numeric constant"))?;
                at = end + 1;
                if active {
                    push(&mut stack, value)?;
                }
            }
            b'\'' => {
                let value = *template
                    .get(at)
                    .ok_or(ExpandError("missing character constant"))?;
                if template.get(at + 1) != Some(&b'\'') {
                    return Err(ExpandError("unterminated character constant"));
                }
                at += 2;
                if active {
                    push(&mut stack, i32::from(value))?;
                }
            }
            b'd' | b'c' | b'2' | b'3' | b'0' | b':' => {
                let (width, zero, format) = match op {
                    b'd' | b'c' => (0usize, false, op),
                    b'2' | b'3' => {
                        let format = *template.get(at).ok_or(ExpandError("missing format"))?;
                        at += 1;
                        (usize::from(op - b'0'), false, format)
                    }
                    _ => {
                        if op == b':' && template.get(at) == Some(&b'0') {
                            at += 1;
                        }
                        let first = *template
                            .get(at)
                            .ok_or(ExpandError("missing format width"))?;
                        at += 1;
                        let width = usize::from(first.saturating_sub(b'0'));
                        let format = *template.get(at).ok_or(ExpandError("missing format"))?;
                        at += 1;
                        (width, true, format)
                    }
                };
                if format != b'd' && format != b'c' || width > 9 {
                    return Err(ExpandError("unsupported numeric format"));
                }
                if active {
                    let value = pop(&mut stack)?;
                    if format == b'c' {
                        let byte = u8::try_from(value)
                            .map_err(|_| ExpandError("character out of range"))?;
                        append(&mut out, &[byte])?;
                    } else {
                        let rendered = if zero {
                            format!("{value:0width$}")
                        } else {
                            format!("{value:width$}")
                        };
                        append(&mut out, rendered.as_bytes())?;
                    }
                }
            }
            b'!' | b'~' => {
                if active {
                    let value = pop(&mut stack)?;
                    push(
                        &mut stack,
                        if op == b'!' {
                            i32::from(value == 0)
                        } else {
                            !value
                        },
                    )?;
                }
            }
            b'+' | b'-' | b'*' | b'/' | b'm' | b'&' | b'|' | b'^' | b'=' | b'>' | b'<' | b'A'
            | b'O' => {
                if active {
                    let right = pop(&mut stack)?;
                    let left = pop(&mut stack)?;
                    let value = match op {
                        b'+' => left.checked_add(right),
                        b'-' => left.checked_sub(right),
                        b'*' => left.checked_mul(right),
                        b'/' => left.checked_div(right),
                        b'm' => left.checked_rem(right),
                        b'&' => Some(left & right),
                        b'|' => Some(left | right),
                        b'^' => Some(left ^ right),
                        b'=' => Some(i32::from(left == right)),
                        b'>' => Some(i32::from(left > right)),
                        b'<' => Some(i32::from(left < right)),
                        b'A' => Some(i32::from(left != 0 && right != 0)),
                        b'O' => Some(i32::from(left != 0 || right != 0)),
                        _ => unreachable!(),
                    }
                    .ok_or(ExpandError("terminfo arithmetic error"))?;
                    push(&mut stack, value)?;
                }
            }
            _ => return Err(ExpandError("unsupported terminfo directive")),
        }
    }
    if !branches.is_empty() {
        return Err(ExpandError("unterminated terminfo conditional"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cursor_parameters_and_order() {
        assert_eq!(
            expand(b"\x1b[%i%p1%d;%p2%dH", [4, 9]).unwrap(),
            b"\x1b[5;10H"
        );
        assert_eq!(expand(b"%p2%d,%p1%d", [4, 9]).unwrap(), b"9,4");
        assert_eq!(expand(b"%p1%02d", [4, 9]).unwrap(), b"04");
    }
    #[test]
    fn conditional_and_malformed() {
        let template = b"%?%p1%{8}%<%tlow%ehi%;";
        assert_eq!(expand(template, [7, 0]).unwrap(), b"low");
        assert_eq!(expand(template, [8, 0]).unwrap(), b"hi");
        for bad in [
            b"%".as_slice(),
            b"%p",
            b"%p3",
            b"%?%p1%t",
            b"%{oops}%d",
            b"%p1%{0}%/%d",
        ] {
            assert!(expand(bad, [1, 2]).is_err(), "{bad:?}");
        }
    }
}
