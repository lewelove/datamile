# Tests for Datamile `d.*` -> Neovim `vim.*` parity

This test module tries to align existing Datamile Lua API with Neovim one, using tests coverage directly from the [Neovim repository](https://github.com/neovim/neovim/tree/master/test/functional/lua). The goal is to provide sane, stable, and most importantly **familiar** API for the Datamile config.

# Procedure

Tests from Neovim repo are copied verbatim in `specs/original/`. Some of them require modding to cut out non-relevant logic. Modded copies reside in `specs/modded/`. The `shim.lua` is used to align the `d.*` function to the `vim.*` ones, while trying to preserve original test logic as much as possible.
