-- Test suite for testing interactions with API bindings
local t = require('test.testutil')
local n = require('test.functional.testnvim')()

local describe, it, before_each = t.describe, t.it, t.before_each
local fn = n.fn
local clear = n.clear
local eq = t.eq
local ok = t.ok
local pcall_err = t.pcall_err
local exec_lua = n.exec_lua
local matches = t.matches
local NIL = vim.NIL

describe('lua stdlib', function()
  before_each(clear)
  -- İ: `tolower("İ")` is `i` which has length 1 while `İ` itself has
  --    length 2 (in bytes).
  -- Ⱥ: `tolower("Ⱥ")` is `ⱥ` which has length 2 while `Ⱥ` itself has
  --    length 3 (in bytes).
  --
  -- Note: 'i' !=? 'İ' and 'ⱥ' !=? 'Ⱥ' on some systems.
  -- Note: Built-in Nvim comparison (on systems lacking `strcasecmp`) works
  --       only on ASCII characters.
  it('vim.stricmp', function()
    eq(0, fn.luaeval('vim.stricmp("a", "A")'))
    eq(0, fn.luaeval('vim.stricmp("A", "a")'))
    eq(0, fn.luaeval('vim.stricmp("a", "a")'))
    eq(0, fn.luaeval('vim.stricmp("A", "A")'))

    eq(0, fn.luaeval('vim.stricmp("", "")'))
    eq(0, fn.luaeval('vim.stricmp("\\0", "\\0")'))
    eq(0, fn.luaeval('vim.stricmp("\\0\\0", "\\0\\0")'))
    eq(0, fn.luaeval('vim.stricmp("\\0\\0\\0", "\\0\\0\\0")'))
    eq(0, fn.luaeval('vim.stricmp("\\0\\0\\0A", "\\0\\0\\0a")'))
    eq(0, fn.luaeval('vim.stricmp("\\0\\0\\0a", "\\0\\0\\0A")'))
    eq(0, fn.luaeval('vim.stricmp("\\0\\0\\0a", "\\0\\0\\0a")'))

    eq(0, fn.luaeval('vim.stricmp("a\\0", "A\\0")'))
    eq(0, fn.luaeval('vim.stricmp("A\\0", "a\\0")'))
    eq(0, fn.luaeval('vim.stricmp("a\\0", "a\\0")'))
    eq(0, fn.luaeval('vim.stricmp("A\\0", "A\\0")'))

    eq(0, fn.luaeval('vim.stricmp("\\0a", "\\0A")'))
    eq(0, fn.luaeval('vim.stricmp("\\0A", "\\0a")'))
    eq(0, fn.luaeval('vim.stricmp("\\0a", "\\0a")'))
    eq(0, fn.luaeval('vim.stricmp("\\0A", "\\0A")'))

    eq(0, fn.luaeval('vim.stricmp("\\0a\\0", "\\0A\\0")'))
    eq(0, fn.luaeval('vim.stricmp("\\0A\\0", "\\0a\\0")'))
    eq(0, fn.luaeval('vim.stricmp("\\0a\\0", "\\0a\\0")'))
    eq(0, fn.luaeval('vim.stricmp("\\0A\\0", "\\0A\\0")'))

    eq(-1, fn.luaeval('vim.stricmp("a", "B")'))
    eq(-1, fn.luaeval('vim.stricmp("A", "b")'))
    eq(-1, fn.luaeval('vim.stricmp("a", "b")'))
    eq(-1, fn.luaeval('vim.stricmp("A", "B")'))

    eq(-1, fn.luaeval('vim.stricmp("", "\\0")'))
    eq(-1, fn.luaeval('vim.stricmp("\\0", "\\0\\0")'))
    eq(-1, fn.luaeval('vim.stricmp("\\0\\0", "\\0\\0\\0")'))
    eq(-1, fn.luaeval('vim.stricmp("\\0\\0\\0A", "\\0\\0\\0b")'))
    eq(-1, fn.luaeval('vim.stricmp("\\0\\0\\0a", "\\0\\0\\0B")'))
    eq(-1, fn.luaeval('vim.stricmp("\\0\\0\\0a", "\\0\\0\\0b")'))

    eq(-1, fn.luaeval('vim.stricmp("a\\0", "B\\0")'))
    eq(-1, fn.luaeval('vim.stricmp("A\\0", "b\\0")'))
    eq(-1, fn.luaeval('vim.stricmp("a\\0", "b\\0")'))
    eq(-1, fn.luaeval('vim.stricmp("A\\0", "B\\0")'))

    eq(-1, fn.luaeval('vim.stricmp("\\0a", "\\0B")'))
    eq(-1, fn.luaeval('vim.stricmp("\\0A", "\\0b")'))
    eq(-1, fn.luaeval('vim.stricmp("\\0a", "\\0b")'))
    eq(-1, fn.luaeval('vim.stricmp("\\0A", "\\0B")'))

    eq(-1, fn.luaeval('vim.stricmp("\\0a\\0", "\\0B\\0")'))
    eq(-1, fn.luaeval('vim.stricmp("\\0A\\0", "\\0b\\0")'))
    eq(-1, fn.luaeval('vim.stricmp("\\0a\\0", "\\0b\\0")'))
    eq(-1, fn.luaeval('vim.stricmp("\\0A\\0", "\\0B\\0")'))

    eq(1, fn.luaeval('vim.stricmp("c", "B")'))
    eq(1, fn.luaeval('vim.stricmp("C", "b")'))
    eq(1, fn.luaeval('vim.stricmp("c", "b")'))
    eq(1, fn.luaeval('vim.stricmp("C", "B")'))

    eq(1, fn.luaeval('vim.stricmp("\\0", "")'))
    eq(1, fn.luaeval('vim.stricmp("\\0\\0", "\\0")'))
    eq(1, fn.luaeval('vim.stricmp("\\0\\0\\0", "\\0\\0")'))
    eq(1, fn.luaeval('vim.stricmp("\\0\\0\\0\\0", "\\0\\0\\0")'))
    eq(1, fn.luaeval('vim.stricmp("\\0\\0\\0C", "\\0\\0\\0b")'))
    eq(1, fn.luaeval('vim.stricmp("\\0\\0\\0c", "\\0\\0\\0B")'))
    eq(1, fn.luaeval('vim.stricmp("\\0\\0\\0c", "\\0\\0\\0b")'))

    eq(1, fn.luaeval('vim.stricmp("c\\0", "B\\0")'))
    eq(1, fn.luaeval('vim.stricmp("C\\0", "b\\0")'))
    eq(1, fn.luaeval('vim.stricmp("c\\0", "b\\0")'))
    eq(1, fn.luaeval('vim.stricmp("C\\0", "B\\0")'))

    eq(1, fn.luaeval('vim.stricmp("c\\0", "B")'))
    eq(1, fn.luaeval('vim.stricmp("C\\0", "b")'))
    eq(1, fn.luaeval('vim.stricmp("c\\0", "b")'))
    eq(1, fn.luaeval('vim.stricmp("C\\0", "B")'))

    eq(1, fn.luaeval('vim.stricmp("\\0c", "\\0B")'))
    eq(1, fn.luaeval('vim.stricmp("\\0C", "\\0b")'))
    eq(1, fn.luaeval('vim.stricmp("\\0c", "\\0b")'))
    eq(1, fn.luaeval('vim.stricmp("\\0C", "\\0B")'))

    eq(1, fn.luaeval('vim.stricmp("\\0c\\0", "\\0B\\0")'))
    eq(1, fn.luaeval('vim.stricmp("\\0C\\0", "\\0b\\0")'))
    eq(1, fn.luaeval('vim.stricmp("\\0c\\0", "\\0b\\0")'))
    eq(1, fn.luaeval('vim.stricmp("\\0C\\0", "\\0B\\0")'))
  end)

  it('vim.startswith', function()
    eq(true, fn.luaeval('vim.startswith("123", "1")'))
    eq(true, fn.luaeval('vim.startswith("123", "")'))
    eq(true, fn.luaeval('vim.startswith("123", "123")'))
    eq(true, fn.luaeval('vim.startswith("", "")'))

    eq(false, fn.luaeval('vim.startswith("123", " ")'))
    eq(false, fn.luaeval('vim.startswith("123", "2")'))
    eq(false, fn.luaeval('vim.startswith("123", "1234")'))

    matches(
      'prefix: expected string, got nil',
      pcall_err(exec_lua, 'return vim.startswith("123", nil)')
    )
    matches('s: expected string, got nil', pcall_err(exec_lua, 'return vim.startswith(nil, "123")'))
  end)

  it('vim.endswith', function()
    eq(true, fn.luaeval('vim.endswith("123", "3")'))
    eq(true, fn.luaeval('vim.endswith("123", "")'))
    eq(true, fn.luaeval('vim.endswith("123", "123")'))
    eq(true, fn.luaeval('vim.endswith("", "")'))

    eq(false, fn.luaeval('vim.endswith("123", " ")'))
    eq(false, fn.luaeval('vim.endswith("123", "2")'))
    eq(false, fn.luaeval('vim.endswith("123", "1234")'))

    matches(
      'suffix: expected string, got nil',
      pcall_err(exec_lua, 'return vim.endswith("123", nil)')
    )
    matches('s: expected string, got nil', pcall_err(exec_lua, 'return vim.endswith(nil, "123")'))
  end)

  it('vim.str_utf_start', function()
    exec_lua([[_G.test_text = "xy åäö ɧ 汉语 ↥ 🤦x🦄 å بِيَّ"]])
    local expected_positions = {
      0,
      0,
      0,
      0,
      -1,
      0,
      -1,
      0,
      -1,
      0,
      0,
      -1,
      0,
      0,
      -1,
      -2,
      0,
      -1,
      -2,
      0,
      0,
      -1,
      -2,
      0,
      0,
      -1,
      -2,
      -3,
      0,
      0,
      -1,
      -2,
      -3,
      0,
      0,
      0,
      -1,
      0,
      0,
      -1,
      0,
      -1,
      0,
      -1,
      0,
      -1,
      0,
      -1,
    }
    eq(
      expected_positions,
      exec_lua([[
      local start_codepoint_positions = {}
      for idx = 1, #_G.test_text do
        table.insert(start_codepoint_positions, vim.str_utf_start(_G.test_text, idx))
      end
      return start_codepoint_positions
    ]])
    )
  end)

  it('vim.str_utf_end', function()
    exec_lua([[_G.test_text = "xy åäö ɧ 汉语 ↥ 🤦x🦄 å بِيَّ"]])
    local expected_positions = {
      0,
      0,
      0,
      1,
      0,
      1,
      0,
      1,
      0,
      0,
      1,
      0,
      0,
      2,
      1,
      0,
      2,
      1,
      0,
      0,
      2,
      1,
      0,
      0,
      3,
      2,
      1,
      0,
      0,
      3,
      2,
      1,
      0,
      0,
      0,
      1,
      0,
      0,
      1,
      0,
      1,
      0,
      1,
      0,
      1,
      0,
      1,
      0,
    }
    eq(
      expected_positions,
      exec_lua([[
      local end_codepoint_positions = {}
      for idx = 1, #_G.test_text do
        table.insert(end_codepoint_positions, vim.str_utf_end(_G.test_text, idx))
      end
      return end_codepoint_positions
    ]])
    )
  end)

  it('vim.str_utf_pos', function()
    exec_lua([[_G.test_text = "xy åäö ɧ 汉语 ↥ 🤦x🦄 å بِيَّ"]])
    local expected_positions = {
      1,
      2,
      3,
      4,
      6,
      8,
      10,
      11,
      13,
      14,
      17,
      20,
      21,
      24,
      25,
      29,
      30,
      34,
      35,
      36,
      38,
      39,
      41,
      43,
      45,
      47,
    }
    eq(expected_positions, exec_lua('return vim.str_utf_pos(_G.test_text)'))
  end)

  it('vim.gsplit, vim.split', function()
    local tests = {
      --                            plain  trimempty
      { 'a,b', ',', false, false, { 'a', 'b' } },
      { ':aa::::bb:', ':', false, false, { '', 'aa', '', '', '', 'bb', '' } },
      { ':aa::::bb:', ':', false, true, { 'aa', '', '', '', 'bb' } },
      { 'aa::::bb:', ':', false, true, { 'aa', '', '', '', 'bb' } },
      { ':aa::bb:', ':', false, true, { 'aa', '', 'bb' } },
      { '/a/b:/b/\n', '[:\n]', false, true, { '/a/b', '/b/' } },
      { '::ee::ff:', ':', false, false, { '', '', 'ee', '', 'ff', '' } },
      { '::ee::ff::', ':', false, true, { 'ee', '', 'ff' } },
      { 'ab', '.', false, false, { '', '', '' } },
      { 'a1b2c', '[0-9]', false, false, { 'a', 'b', 'c' } },
      { 'xy', '', false, false, { 'x', 'y' } },
      { 'here be dragons', ' ', false, false, { 'here', 'be', 'dragons' } },
      { 'axaby', 'ab?', false, false, { '', 'x', 'y' } },
      { 'f v2v v3v w2w ', '([vw])2%1', false, false, { 'f ', ' v3v ', ' ' } },
      { '', '', false, false, {} },
      { '', '', false, true, {} },
      { '\n', '[:\n]', false, true, {} },
      { '', 'a', false, false, { '' } },
      { 'x*yz*oo*l', '*', true, false, { 'x', 'yz', 'oo', 'l' } },
    }

    for _, q in ipairs(tests) do
      eq(q[5], vim.split(q[1], q[2], { plain = q[3], trimempty = q[4] }), q[1])
    end

    -- Test old signature
    eq({ 'x', 'yz', 'oo', 'l' }, vim.split('x*yz*oo*l', '*', true))

    local loops = {
      { 'abc', '.-' },
    }

    for _, q in ipairs(loops) do
      matches('Infinite loop detected', pcall_err(vim.split, q[1], q[2]))
    end

    -- Validates args.
    eq(true, pcall(vim.split, 'string', 'string'))
    matches('s: expected string, got number', pcall_err(vim.split, 1, 'string'))
    matches('sep: expected string, got number', pcall_err(vim.split, 'string', 1))
    matches('opts: expected table, got number', pcall_err(vim.split, 'string', 'string', 1))
  end)

  it('vim.trim', function()
    local trim = function(s)
      return exec_lua('return vim.trim(...)', s)
    end

    local trims = {
      { '   a', 'a' },
      { ' b  ', 'b' },
      { '\tc', 'c' },
      { 'r\n', 'r' },
      { '', '' },
      { ' \t \n', '' },
    }

    for _, q in ipairs(trims) do
      eq(q[2], trim(q[1]))
    end

    -- Validates args.
    matches('s: expected string, got number', pcall_err(trim, 2))
  end)

  it('vim.deepcopy', function()
    ok(exec_lua([[
      local a = { x = { 1, 2 }, y = 5}
      local b = vim.deepcopy(a)

      return b.x[1] == 1 and b.x[2] == 2 and b.y == 5 and vim.tbl_count(b) == 2
             and tostring(a) ~= tostring(b)
    ]]))

    ok(exec_lua([[
      local a = {}
      local b = vim.deepcopy(a)

      return vim.islist(b) and vim.tbl_count(b) == 0 and tostring(a) ~= tostring(b)
    ]]))

    ok(exec_lua([[
      local a = vim.empty_dict()
      local b = vim.deepcopy(a)

      return not vim.islist(b) and vim.tbl_count(b) == 0
    ]]))

    ok(exec_lua([[
      local a = {x = vim.empty_dict(), y = {}}
      local b = vim.deepcopy(a)

      return not vim.islist(b.x) and vim.islist(b.y)
        and vim.tbl_count(b) == 2
        and tostring(a) ~= tostring(b)
    ]]))

    ok(exec_lua([[
      local f1 = function() return 1 end
      local f2 = function() return 2 end
      local t1 = {f = f1}
      local t2 = vim.deepcopy(t1)
      t1.f = f2
      return t1.f() ~= t2.f()
    ]]))

    ok(exec_lua([[
      local t1 = {a = 5}
      t1.self = t1
      local t2 = vim.deepcopy(t1)
      return t2.self == t2 and t2.self ~= t1
    ]]))

    ok(exec_lua([[
      local mt = {mt=true}
      local t1 = setmetatable({a = 5}, mt)
      local t2 = vim.deepcopy(t1)
      return getmetatable(t2) == mt
    ]]))

    ok(exec_lua([[
      local t1 = {a = vim.NIL}
      local t2 = vim.deepcopy(t1)
      return t2.a == vim.NIL
    ]]))
  end)

  it('vim._copy', function()
    ok(exec_lua([[
      local inner = { x = 1 }
      local mt = { tag = true }
      local a = setmetatable({ inner = inner }, mt)
      local b = vim._copy(a)

      local c = vim.empty_dict()
      c.inner = inner
      local d = vim._copy(c)

      return b ~= a
        and b.inner == inner
        and getmetatable(b) == mt
        and d ~= c
        and d.inner == inner
        and not vim.islist(d)
    ]]))
  end)

  it('vim.pesc', function()
    eq('foo%-bar', exec_lua([[return vim.pesc('foo-bar')]]))
    eq('foo%%%-bar', exec_lua([[return vim.pesc(vim.pesc('foo-bar'))]]))
    -- pesc() returns one result. #20751
    eq({ 'x' }, exec_lua([[return {vim.pesc('x')}]]))

    -- Validates args.
    matches('s: expected string, got number', pcall_err(exec_lua, [[return vim.pesc(2)]]))
  end)

  it('vim.list_contains', function()
    eq(true, exec_lua("return vim.list_contains({'a','b','c'}, 'c')"))
    eq(false, exec_lua("return vim.list_contains({'a','b','c'}, 'd')"))
  end)

  it('vim.tbl_contains', function()
    eq(true, exec_lua("return vim.tbl_contains({'a','b','c'}, 'c')"))
    eq(false, exec_lua("return vim.tbl_contains({'a','b','c'}, 'd')"))
    eq(true, exec_lua("return vim.tbl_contains({[2]='a',foo='b',[5] = 'c'}, 'c')"))
    eq(
      true,
      exec_lua([[
        return vim.tbl_contains({ 'a', { 'b', 'c' } }, function(v)
          return vim.deep_equal(v, { 'b', 'c' })
        end, { predicate = true })
    ]])
    )
  end)

  it('vim.tbl_keys', function()
    eq({}, exec_lua('return vim.tbl_keys({})'))
    for _, v in pairs(exec_lua("return vim.tbl_keys({'a', 'b', 'c'})")) do
      eq(true, exec_lua('return vim.tbl_contains({ 1, 2, 3 }, ...)', v))
    end
    for _, v in pairs(exec_lua('return vim.tbl_keys({a=1, b=2, c=3})')) do
      eq(true, exec_lua("return vim.tbl_contains({ 'a', 'b', 'c' }, ...)", v))
    end
  end)

  it('vim.tbl_values', function()
    eq({}, exec_lua('return vim.tbl_values({})'))
    for _, v in pairs(exec_lua("return vim.tbl_values({'a', 'b', 'c'})")) do
      eq(true, exec_lua("return vim.tbl_contains({ 'a', 'b', 'c' }, ...)", v))
    end
    for _, v in pairs(exec_lua('return vim.tbl_values({a=1, b=2, c=3})')) do
      eq(true, exec_lua('return vim.tbl_contains({ 1, 2, 3 }, ...)', v))
    end
  end)

  it('vim.tbl_map', function()
    eq(
      {},
      exec_lua([[
      return vim.tbl_map(function(v) return v * 2 end, {})
    ]])
    )
    eq(
      { 2, 4, 6 },
      exec_lua([[
      return vim.tbl_map(function(v) return v * 2 end, {1, 2, 3})
    ]])
    )
    eq(
      { { i = 2 }, { i = 4 }, { i = 6 } },
      exec_lua([[
      return vim.tbl_map(function(v) return { i = v.i * 2 } end, {{i=1}, {i=2}, {i=3}})
    ]])
    )
  end)

  it('vim.tbl_filter', function()
    eq(
      {},
      exec_lua([[
      return vim.tbl_filter(function(v) return (v % 2) == 0 end, {})
    ]])
    )
    eq(
      { 2 },
      exec_lua([[
      return vim.tbl_filter(function(v) return (v % 2) == 0 end, {1, 2, 3})
    ]])
    )
    eq(
      { { i = 2 } },
      exec_lua([[
      return vim.tbl_filter(function(v) return (v.i % 2) == 0 end, {{i=1}, {i=2}, {i=3}})
    ]])
    )
  end)

  it('vim.isarray', function()
    eq(true, exec_lua('return vim.isarray({})'))
    eq(false, exec_lua('return vim.isarray(vim.empty_dict())'))
    eq(true, exec_lua("return vim.isarray({'a', 'b', 'c'})"))
    eq(false, exec_lua("return vim.isarray({'a', '32', a='hello', b='baz'})"))
    eq(false, exec_lua("return vim.isarray({1, a='hello', b='baz'})"))
    eq(false, exec_lua("return vim.isarray({a='hello', b='baz', 1})"))
    eq(false, exec_lua("return vim.isarray({1, 2, nil, a='hello'})"))
    eq(true, exec_lua('return vim.isarray({1, 2, nil, 4})'))
    eq(true, exec_lua('return vim.isarray({nil, 2, 3, 4})'))
    eq(false, exec_lua('return vim.isarray({1, [1.5]=2, [3]=3})'))
  end)

  it('vim.islist', function()
    eq(true, exec_lua('return vim.islist({})'))
    eq(false, exec_lua('return vim.islist(vim.empty_dict())'))
    eq(true, exec_lua("return vim.islist({'a', 'b', 'c'})"))
    eq(false, exec_lua("return vim.islist({'a', '32', a='hello', b='baz'})"))
    eq(false, exec_lua("return vim.islist({1, a='hello', b='baz'})"))
    eq(false, exec_lua("return vim.islist({a='hello', b='baz', 1})"))
    eq(false, exec_lua("return vim.islist({1, 2, nil, a='hello'})"))
    eq(false, exec_lua('return vim.islist({1, 2, nil, 4})'))
    eq(false, exec_lua('return vim.islist({nil, 2, 3, 4})'))
    eq(false, exec_lua('return vim.islist({1, [1.5]=2, [3]=3})'))
  end)

  it('vim.isnil', function()
    eq(true, exec_lua('return vim.isnil(nil)'))
    eq(true, exec_lua('return vim.isnil(vim.NIL)'))
    eq(false, exec_lua('return vim.isnil(true)'))
    eq(false, exec_lua('return vim.isnil(false)'))
    eq(false, exec_lua('return vim.isnil({})'))
  end)

  it('vim.tbl_isempty', function()
    eq(true, exec_lua('return vim.tbl_isempty({})'))
    eq(false, exec_lua('return vim.tbl_isempty({ 1, 2, 3 })'))
    eq(false, exec_lua('return vim.tbl_isempty({a=1, b=2, c=3})'))
  end)

  it('vim.tbl_get', function()
    eq(
      true,
      exec_lua("return vim.tbl_get({ test = { nested_test = true }}, 'test', 'nested_test')")
    )
    eq(NIL, exec_lua("return vim.tbl_get({ unindexable = true }, 'unindexable', 'missing_key')"))
    eq(NIL, exec_lua("return vim.tbl_get({ unindexable = 1 }, 'unindexable', 'missing_key')"))
    eq(NIL, exec_lua("return vim.tbl_get({}, 'missing_key')"))
    eq(NIL, exec_lua('return vim.tbl_get({})'))
    eq(NIL, exec_lua("return vim.tbl_get({}, nil, 'key')"))
  end)

  it('vim.tbl_extend', function()
    ok(exec_lua([[
      local a = {x = 1}
      local b = {y = 2}
      local c = vim.tbl_extend("keep", a, b)

      return c.x == 1 and b.y == 2 and vim.tbl_count(c) == 2
    ]]))

    ok(exec_lua([[
      local a = {x = 1}
      local b = {y = 2}
      local c = {z = 3}
      local d = vim.tbl_extend("keep", a, b, c)

      return d.x == 1 and d.y == 2 and d.z == 3 and vim.tbl_count(d) == 3
    ]]))

    ok(exec_lua([[
      local a = {x = 1}
      local b = {x = 3}
      local c = vim.tbl_extend("keep", a, b)

      return c.x == 1 and vim.tbl_count(c) == 1
    ]]))

    ok(exec_lua([[
      local a = {x = 1}
      local b = {x = 3}
      local c = vim.tbl_extend("force", a, b)

      return c.x == 3 and vim.tbl_count(c) == 1
    ]]))

    ok(exec_lua([[
      local a = vim.empty_dict()
      local b = {}
      local c = vim.tbl_extend("keep", a, b)

      return not vim.islist(c) and vim.tbl_count(c) == 0
    ]]))

    ok(exec_lua([[
      local a = {}
      local b = vim.empty_dict()
      local c = vim.tbl_extend("keep", a, b)

      return vim.islist(c) and vim.tbl_count(c) == 0
    ]]))

    ok(exec_lua([[
      local a = {x = {a = 1, b = 2}}
      local b = {x = {a = 2, c = {y = 3}}}
      local c = vim.tbl_extend("keep", a, b)

      local count = 0
      for _ in pairs(c) do count = count + 1 end

      return c.x.a == 1 and c.x.b == 2 and c.x.c == nil and count == 1
    ]]))

    ok(exec_lua([[
      local a = { a = 1, b = 2, c = 1 }
      local b = { a = -1, b = 5, c = 3, d = 4 }
      -- Return the maximum value for each key.
      local c = vim.tbl_extend(function(k, prev_v, v)
        if prev_v then
          return v > prev_v and v or prev_v
        else
          return v
        end
      end, a, b)
      return vim.deep_equal(c, { a = 1, b = 5, c = 3, d = 4 })
    ]]))
  end)

  it('vim.tbl_deep_extend', function()
    ok(exec_lua([[
      local a = {x = {a = 1, b = 2}}
      local b = {x = {a = 2, c = {y = 3}}}
      local c = vim.tbl_deep_extend("keep", a, b)

      local count = 0
      for _ in pairs(c) do count = count + 1 end

      return c.x.a == 1 and c.x.b == 2 and c.x.c.y == 3 and count == 1
    ]]))

    ok(exec_lua([[
      local a = {x = {a = 1, b = 2}}
      local b = {x = {a = 2, c = {y = 3}}}
      local c = vim.tbl_deep_extend("force", a, b)

      local count = 0
      for _ in pairs(c) do count = count + 1 end

      return c.x.a == 2 and c.x.b == 2 and c.x.c.y == 3 and count == 1
    ]]))

    ok(exec_lua([[
      local a = {x = {a = 1, b = 2}}
      local b = {x = {a = 2, c = {y = 3}}}
      local c = {x = {c = 4, d = {y = 4}}}
      local d = vim.tbl_deep_extend("keep", a, b, c)

      local count = 0
      for _ in pairs(c) do count = count + 1 end

      return d.x.a == 1 and d.x.b == 2 and d.x.c.y == 3 and d.x.d.y == 4 and count == 1
    ]]))

    ok(exec_lua([[
      local a = {x = {a = 1, b = 2}}
      local b = {x = {a = 2, c = {y = 3}}}
      local c = {x = {c = 4, d = {y = 4}}}
      local d = vim.tbl_deep_extend("force", a, b, c)

      local count = 0
      for _ in pairs(c) do count = count + 1 end

      return d.x.a == 2 and d.x.b == 2 and d.x.c == 4 and d.x.d.y == 4 and count == 1
    ]]))

    ok(exec_lua([[
      local a = vim.empty_dict()
      local b = {}
      local c = vim.tbl_deep_extend("keep", a, b)

      local count = 0
      for _ in pairs(c) do count = count + 1 end

      return not vim.islist(c) and count == 0
    ]]))

    ok(exec_lua([[
      local a = {}
      local b = vim.empty_dict()
      local c = vim.tbl_deep_extend("keep", a, b)

      local count = 0
      for _ in pairs(c) do count = count + 1 end

      return vim.islist(c) and count == 0
    ]]))

    eq(
      { a = { b = 1 } },
      exec_lua([[
      local a = { a = { b = 1 } }
      local b = { a = {} }
      return vim.tbl_deep_extend("force", a, b)
    ]])
    )

    eq(
      { a = { b = 1 } },
      exec_lua([[
      local a = { a = 123 }
      local b = { a = { b = 1} }
      return vim.tbl_deep_extend("force", a, b)
    ]])
    )

    ok(exec_lua([[
      local a = { a = {[2] = 3} }
      local b = { a = {[3] = 3} }
      local c = vim.tbl_deep_extend("force", a, b)
      return vim.deep_equal(c, {a = {[2] = 3, [3] = 3}})
    ]]))

    eq(
      { a = 123 },
      exec_lua([[
      local a = { a = { b = 1} }
      local b = { a = 123 }
      return vim.tbl_deep_extend("force", a, b)
    ]])
    )

    ok(exec_lua([[
      local a = { sub = { 'a', 'b' } }
      local b = { sub = { 'b', 'c' } }
      local c = vim.tbl_deep_extend('force', a, b)
      return vim.deep_equal(c, { sub = { 'b', 'c' } })
    ]]))

    ok(exec_lua([[
      local a = { a = 1, b = 2, c = { d = 1, e = -2} }
      local b = { a = -1, b = 5, c = { d = 6 } }
      -- Return the maximum value for each key.
      local c = vim.tbl_deep_extend(function(k, prev_v, v)
        if prev_v then
          return v > prev_v and v or prev_v
        else
          return v
        end
      end, a, b)
      return vim.deep_equal(c, { a = 1, b = 5, c = { d = 6, e = -2 } })
    ]]))
  end)

  it('vim.tbl_count', function()
    eq(0, exec_lua [[ return vim.tbl_count({}) ]])
    eq(0, exec_lua [[ return vim.tbl_count(vim.empty_dict()) ]])
    eq(0, exec_lua [[ return vim.tbl_count({nil}) ]])
    eq(0, exec_lua [[ return vim.tbl_count({a=nil}) ]])
    eq(1, exec_lua [[ return vim.tbl_count({1}) ]])
    eq(2, exec_lua [[ return vim.tbl_count({1, 2}) ]])
    eq(2, exec_lua [[ return vim.tbl_count({1, nil, 3}) ]])
    eq(1, exec_lua [[ return vim.tbl_count({a=1}) ]])
    eq(2, exec_lua [[ return vim.tbl_count({a=1, b=2}) ]])
    eq(2, exec_lua [[ return vim.tbl_count({a=1, b=nil, c=3}) ]])
  end)

  it('vim.deep_equal', function()
    eq(true, exec_lua [[ return vim.deep_equal({a=1}, {a=1}) ]])
    eq(true, exec_lua [[ return vim.deep_equal({a={b=1}}, {a={b=1}}) ]])
    eq(true, exec_lua [[ return vim.deep_equal({a={b={nil}}}, {a={b={}}}) ]])
    eq(true, exec_lua [[ return vim.deep_equal({a=1, [5]=5}, {nil,nil,nil,nil,5,a=1}) ]])
    eq(
      true,
      exec_lua [[ local shared = {}; return vim.deep_equal({ 1, shared, 1, shared }, { 1, {}, 1, {} }) ]]
    )
    -- cyclic table
    eq(true, exec_lua [[ local a,b={},{}; a[1]=a; b[1]=b; return vim.deep_equal(a, b) ]])
    eq(false, exec_lua [[ return vim.deep_equal(1, {nil,nil,nil,nil,5,a=1}) ]])
    eq(false, exec_lua [[ return vim.deep_equal(1, 3) ]])
    eq(false, exec_lua [[ return vim.deep_equal(nil, 3) ]])
    eq(false, exec_lua [[ return vim.deep_equal({a=1}, {a=2}) ]])
    eq(false, exec_lua [[ local a,b={},{}; a[1]=a; b[1]={}; return vim.deep_equal(a, b) ]])
  end)

  it('vim.list_extend', function()
    eq({ 1, 2, 3 }, exec_lua [[ return vim.list_extend({1}, {2,3}) ]])
    matches(
      'src: expected table, got nil',
      pcall_err(exec_lua, [[ return vim.list_extend({1}, nil) ]])
    )
    eq({ 1, 2 }, exec_lua [[ return vim.list_extend({1}, {2;a=1}) ]])
    eq(true, exec_lua [[ local a = {1} return vim.list_extend(a, {2;a=1}) == a ]])
    eq({ 2 }, exec_lua [[ return vim.list_extend({}, {2;a=1}, 1) ]])
    eq({}, exec_lua [[ return vim.list_extend({}, {2;a=1}, 2) ]])
    eq({}, exec_lua [[ return vim.list_extend({}, {2;a=1}, 1, -1) ]])
    eq({ 2 }, exec_lua [[ return vim.list_extend({}, {2;a=1}, -1, 2) ]])
  end)

  it('vim.spairs', function()
    local res = ''
    local table = {
      ccc = 1,
      bbb = 2,
      ddd = 3,
      aaa = 4,
    }
    for key, _ in vim.spairs(table) do
      res = res .. key
    end
    matches('aaabbbcccddd', res)
  end)

  it('vim.empty_dict()', function()
    eq(
      { true, false, true, true },
      exec_lua([[
      local listy = {}
      local dicty = vim.empty_dict()
      return {vim.islist(listy), vim.islist(dicty), next(listy) == nil, next(dicty) == nil}
    ]])
    )
  end)
end)
