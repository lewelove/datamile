# Coding Standards

- Avoid magic numbers and strings by extracting recurring or meaningful values into descriptive constants (`const`) or enums. Keep self-explanatory, one-off values inline to avoid clutter. If a value comes from a spec (e.g. HTTP 200 OK), use a constant regardless.

- Reduce code indentation. Avoid Arrow Anti-Pattern. Leverage early return and continue.

- Use enums instead of booleans for function parameters.

- Let the reader of the code breathe. Add empty lines between logical blocks of code.

- Always document tests. Be concise and to the point.

- ALWAYS document `#[allow]` blocks. Use them only if good reason exists. No exceptions.

- Add a small, to the point, comment to explain *why* the block does what it does. Use examples when possible.

- Use descriptive self-documenting function names that make clear what the function does from the name only.

- Program to levels of abstraction. Lower-level mechanics (e.g., raw hardware I/O, sector parsing, direct socket streams) must be encapsulated in a dedicated driver/abstraction layer. Expose clean, high-level APIs to the rest of the application so calling code works with domain concepts, not raw implementation details.

- Don't touch blocks of code unrelated to the feature you implement. e.g. Don't add comments to a block of code if you did not create it or modify it. As much as possible try to minimize the number of changed lines when implementing a feature.

- Strictly adhere to the layered boundary hierarchy: each layer may only communicate with its immediate neighbor directly below it. Never "punch holes" through layers (e.g., controllers or UI components must never directly call database queries, raw hardware drivers, or low-level network clients; always route through the intermediate service/abstraction layer).

- Always document Rust tests in files dedicated to them. Be concise and to the point.

- Always use `{}`, even on a one-line `if` statement.

- If a bug is found while you are working, or the prompt itself indicates that a bug is being fixed, don't write the fix right away. First write the test. Observe it failing. Then write the fix. And then observe the test passing.
