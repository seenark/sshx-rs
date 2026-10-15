//! Shared by the PTY suites: ratatui redraws with cursor moves and partial erases, so raw
//! substring waits are flaky; replaying the stream onto a character grid shows what the
//! user sees.

/// Replays the escape stream onto a `rows` x `columns` grid and returns its text without
/// whitespace, so layout never matters. Handles the cursor moves, erases and
/// alternate-screen switch ratatui emits.
pub fn render(raw: &[u8], rows: usize, columns: usize) -> String {
    let blank = || vec![vec![' '; columns]; rows];
    let mut grid = blank();
    let (mut row, mut column) = (0usize, 0usize);
    let text = String::from_utf8_lossy(raw);
    let mut characters = text.chars();
    while let Some(character) = characters.next() {
        match character {
            '\x1b' => match characters.next() {
                Some('[') => {
                    let mut parameters = String::new();
                    let mut last = ' ';
                    for next in characters.by_ref() {
                        if ('\x40'..='\x7e').contains(&next) {
                            last = next;
                            break;
                        }
                        parameters.push(next);
                    }
                    let numbers = parameters
                        .split(';')
                        .map(|part| part.trim_start_matches('?').parse::<usize>().unwrap_or(0))
                        .collect::<Vec<_>>();
                    let first = numbers.first().copied().unwrap_or(0).max(1);
                    match last {
                        'H' | 'f' => {
                            row = first - 1;
                            column = numbers.get(1).copied().unwrap_or(0).max(1) - 1;
                        }
                        'A' => row = row.saturating_sub(first),
                        'B' => row += first,
                        'C' => column += first,
                        'D' => column = column.saturating_sub(first),
                        'J' => grid = blank(),
                        'K' => {
                            if let Some(line) = grid.get_mut(row) {
                                line.iter_mut().skip(column).for_each(|cell| *cell = ' ');
                            }
                        }
                        'h' if parameters == "?1049" => grid = blank(),
                        _ => {}
                    }
                }
                Some(']') => {
                    for next in characters.by_ref() {
                        if next == '\x07' {
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\r' => column = 0,
            '\n' => row += 1,
            control if control.is_control() => {}
            printable => {
                if let Some(cell) = grid.get_mut(row).and_then(|line| line.get_mut(column)) {
                    *cell = printable;
                }
                column += 1;
            }
        }
    }
    grid.into_iter()
        .flatten()
        .filter(|character| !character.is_whitespace())
        .collect()
}
