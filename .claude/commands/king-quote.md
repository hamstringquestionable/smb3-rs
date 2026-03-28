Format a quote and add it to the king quotes database in `src/randomize/king_quotes.rs`.

## Usage
`/king-quote <text>`

Examples:
- `/king-quote I tried to warn them about the wizard but nobody listens to a frog.`
- `/king-quote You saved me! Now can you help me find my car keys?`

## Instructions

1. **Take the input text** and word-wrap it into exactly 6 lines, each at most 20 characters.

2. **Wrapping rules:**
   - Break on word boundaries only (never split a word across lines).
   - If a single word exceeds 20 characters, reject the quote and explain.
   - Distribute text across lines naturally — don't front-load or back-load.
   - Trailing lines that aren't needed should be empty strings `""`.
   - No trailing spaces in the string literals (the encoder pads with spaces).

3. **Character validation.** Only these characters are allowed in SMB3's main text table:
   - Letters: `A-Z`, `a-z`
   - Punctuation: `,` `.` `'` `!` `?`
   - Space
   - **NOT allowed:** numbers, dashes, colons, semicolons, quotes `"`, parentheses, or any other symbols.
   - If the input contains disallowed characters, suggest replacements (e.g., `"` → remove, `-` → `,` or reword, numbers → spell out if short enough).

4. **Format as a Rust array literal** matching the existing entries in the `QUOTES` constant:
   ```rust
       [
           "Line one here",
           "line two here",
           "and line three.",
           "",
           "Here is a letter",
           "from the Princess.",
       ],
   ```

5. **Show the formatted quote** to the user for review. Display it both as the Rust literal and as a visual preview:
   ```
   |Line one here       |
   |line two here       |
   |and line three.     |
   |                    |
   |Here is a letter    |
   |from the Princess.  |
   ```

6. **After user approval**, append the new entry to the **standard `QUOTES`** array (the first and largest `&[[&str; 6]]` constant, starting around line 46) in `src/randomize/king_quotes.rs`. Insert it as the last entry before that array's closing `];`. Do NOT add it to the suit-specific arrays (`FROG_QUOTES`, `RACCOON_QUOTES`, `HAMMER_QUOTES`).

7. **Run the tests** to validate: `nix-shell -p gcc --run 'export PATH="$HOME/.cargo/bin:$PATH" && cargo test king_quotes'`

8. If tests fail (line too long, invalid char), fix and re-run.
