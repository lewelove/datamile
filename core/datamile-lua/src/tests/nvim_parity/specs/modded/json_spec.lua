local t = require('test.testutil')
local n = require('test.functional.testnvim')()

local describe, it, before_each = t.describe, t.it, t.before_each
local clear = n.clear
local exec_lua = n.exec_lua
local eq = t.eq
local pcall_err = t.pcall_err

describe('vim.json.decode()', function()
  before_each(function()
    clear()
  end)

  it('parses null, true, false', function()
    eq(vim.NIL, exec_lua([[return vim.json.decode('null')]]))
    eq(true, exec_lua([[return vim.json.decode('true')]]))
    eq(false, exec_lua([[return vim.json.decode('false')]]))
  end)

  it('parses integer numbers', function()
    eq(100000, exec_lua([[return vim.json.decode('100000')]]))
    eq(-100000, exec_lua([[return vim.json.decode('-100000')]]))
    eq(100000, exec_lua([[return vim.json.decode('  100000  ')]]))
    eq(-100000, exec_lua([[return vim.json.decode('  -100000  ')]]))
    eq(0, exec_lua([[return vim.json.decode('0')]]))
    eq(0, exec_lua([[return vim.json.decode('-0')]]))
    eq(3053700806959403, exec_lua([[return vim.json.decode('3053700806959403')]]))
  end)

  it('parses floating-point numbers', function()
    -- This behavior differs from vim.fn.json_decode, which return '100000.0'
    eq('100000', exec_lua([[return tostring(vim.json.decode('100000.0'))]]))
    eq(100000.5, exec_lua([[return vim.json.decode('100000.5')]]))
    eq(-100000.5, exec_lua([[return vim.json.decode('-100000.5')]]))
    eq(-100000.5e50, exec_lua([[return vim.json.decode('-100000.5e50')]]))
    eq(100000.5e50, exec_lua([[return vim.json.decode('100000.5e50')]]))
    eq(100000.5e50, exec_lua([[return vim.json.decode('100000.5e+50')]]))
    eq(-100000.5e-50, exec_lua([[return vim.json.decode('-100000.5e-50')]]))
    eq(100000.5e-50, exec_lua([[return vim.json.decode('100000.5e-50')]]))
    eq(100000e-50, exec_lua([[return vim.json.decode('100000e-50')]]))
    eq(0.5, exec_lua([[return vim.json.decode('0.5')]]))
    eq(0.005, exec_lua([[return vim.json.decode('0.005')]]))
    eq(0.005, exec_lua([[return vim.json.decode('0.00500')]]))
    eq(0.5, exec_lua([[return vim.json.decode('0.00500e+002')]]))
    eq(0.00005, exec_lua([[return vim.json.decode('0.00500e-002')]]))

    eq(-0.0, exec_lua([[return vim.json.decode('-0.0')]]))
    eq(-0.0, exec_lua([[return vim.json.decode('-0.0e0')]]))
    eq(-0.0, exec_lua([[return vim.json.decode('-0.0e+0')]]))
    eq(-0.0, exec_lua([[return vim.json.decode('-0.0e-0')]]))
    eq(-0.0, exec_lua([[return vim.json.decode('-0e-0')]]))
    eq(-0.0, exec_lua([[return vim.json.decode('-0e-2')]]))
    eq(-0.0, exec_lua([[return vim.json.decode('-0e+2')]]))

    eq(0.0, exec_lua([[return vim.json.decode('0.0')]]))
    eq(0.0, exec_lua([[return vim.json.decode('0.0e0')]]))
    eq(0.0, exec_lua([[return vim.json.decode('0.0e+0')]]))
    eq(0.0, exec_lua([[return vim.json.decode('0.0e-0')]]))
    eq(0.0, exec_lua([[return vim.json.decode('0e-0')]]))
    eq(0.0, exec_lua([[return vim.json.decode('0e-2')]]))
    eq(0.0, exec_lua([[return vim.json.decode('0e+2')]]))
  end)

  it('parses containers', function()
    eq({ 1 }, exec_lua([[return vim.json.decode('[1]')]]))
    eq({ vim.NIL, 1 }, exec_lua([[return vim.json.decode('[null, 1]')]]))
    eq({ ['1'] = 2 }, exec_lua([[return vim.json.decode('{"1": 2}')]]))
    eq(
      { ['1'] = 2, ['3'] = { { ['4'] = { ['5'] = { {}, 1 } } } } },
      exec_lua([[return vim.json.decode('{"1": 2, "3": [{"4": {"5": [ [], 1]}}]}')]])
    )
    -- Empty string is a valid key. #20757
    eq({ [''] = 42 }, exec_lua([[return vim.json.decode('{"": 42}')]]))
  end)

  it('parses strings properly', function()
    eq('\n', exec_lua([=[return vim.json.decode([["\n"]])]=]))
    eq('', exec_lua([=[return vim.json.decode([[""]])]=]))
    eq('\\/"\t\b\n\r\f', exec_lua([=[return vim.json.decode([["\\\/\"\t\b\n\r\f"]])]=]))
    eq('/a', exec_lua([=[return vim.json.decode([["\/a"]])]=]))
    -- Unicode characters: 2-byte, 3-byte
    eq('«', exec_lua([=[return vim.json.decode([["«"]])]=]))
    eq('ફ', exec_lua([=[return vim.json.decode([["ફ"]])]=]))
  end)

  it('accepts all spaces in every position where space may be put', function()
    local s =
      ' \t\n\r \t\r\n \n\t\r \n\r\t \r\t\n \r\n\t\t \n\r\t \r\n\t\n \r\t\n\r \t\r \n\t\r\n \n \t\r\n \r\t\n\t \r\n\t\r \n\r \t\n\r\t \r \t\n\r \n\t\r\t \n\r\t\n \r\n \t\r\n\t'
    local str = ('%s{%s"key"%s:%s[%s"val"%s,%s"val2"%s]%s,%s"key2"%s:%s1%s}%s'):gsub('%%s', s)
    eq({ key = { 'val', 'val2' }, key2 = 1 }, exec_lua([[return vim.json.decode(...)]], str))
  end)
end)

describe('vim.json.encode()', function()
  before_each(function()
    clear()
  end)

  it('dumps strings', function()
    eq('"Test"', exec_lua([[return vim.json.encode('Test')]]))
    eq('""', exec_lua([[return vim.json.encode('')]]))
    eq('"\\t"', exec_lua([[return vim.json.encode('\t')]]))
    eq('"\\n"', exec_lua([[return vim.json.encode('\n')]]))
  end)

  it('dumps numbers', function()
    eq('0', exec_lua([[return vim.json.encode(0)]]))
    eq('10', exec_lua([[return vim.json.encode(10)]]))
    eq('-10', exec_lua([[return vim.json.encode(-10)]]))
  end)

  it('dumps floats', function()
    eq('3053700806959403', exec_lua([[return vim.json.encode(3053700806959403)]]))
    eq('10.5', exec_lua([[return vim.json.encode(10.5)]]))
    eq('-10.5', exec_lua([[return vim.json.encode(-10.5)]]))
  end)

  it('dumps lists', function()
    eq('[]', exec_lua([[return vim.json.encode({})]]))
    eq('[[]]', exec_lua([[return vim.json.encode({{}})]]))
    eq('[[],[]]', exec_lua([[return vim.json.encode({{}, {}})]]))
  end)

  it('dumps dictionaries', function()
    eq('{}', exec_lua([[return vim.json.encode(vim.empty_dict())]]))
    eq('{"d":[]}', exec_lua([[return vim.json.encode({d={}})]]))
    -- Empty string is a valid key. #20757
    eq('{"":42}', exec_lua([[return vim.json.encode({['']=42})]]))
  end)

  it('dumps vim.NIL', function()
    eq('null', exec_lua([[return vim.json.encode(vim.NIL)]]))
  end)
end)
