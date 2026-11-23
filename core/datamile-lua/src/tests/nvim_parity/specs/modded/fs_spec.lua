local t = require('test.testutil')
local n = require('test.functional.testnvim')()

local describe, it, before_each, after_each, setup =
  t.describe, t.it, t.before_each, t.after_each, t.setup
local clear = n.clear
local exec_lua = n.exec_lua
local eq = t.eq
local eq_paths = t.eq_paths
local mkdir_p = n.mkdir_p
local rmdir = n.rmdir
local nvim_dir = n.nvim_dir
local test_build_dir = t.paths.test_build_dir
local test_source_path = t.paths.test_source_path
local nvim_prog = n.nvim_prog
local is_os = t.is_os
local mkdir = t.mkdir

local nvim_prog_basename = is_os('win') and 'nvim.exe' or 'nvim'

local test_basename_dirname_eq = {
  '~/foo/',
  '~/foo',
  '~/foo/bar.lua',
  'foo.lua',
  ' ',
  '',
  '.',
  '..',
  '../',
  '~',
  '/usr/bin',
  '/usr/bin/gcc',
  '/',
  '/usr/',
  '/usr',
}

setup(clear)

describe('vim.fs', function()
  describe('parents()', function()
    it('works', function()
      local test_dir = nvim_dir .. '/test'
      mkdir_p(test_dir)
      local dirs = {} --- @type string[]
      for dir in vim.fs.parents(test_dir .. '/foo.txt') do
        dirs[#dirs + 1] = dir
        if dir == test_build_dir then
          break
        end
      end
      eq({ test_dir, nvim_dir, test_build_dir }, dirs)
      rmdir(test_dir)
    end)
  end)

  describe('dirname()', function()
    it('works', function()
      eq(test_build_dir, vim.fs.dirname(nvim_dir))

      ---@param paths string[]
      ---@param is_win? boolean
      local function test_paths(paths, is_win)
        local gsub = is_win and [[:gsub('\\', '/')]] or ''
        local code = string.format(
          [[
          local path = ...
          return vim.fn.fnamemodify(path,':h')%s
        ]],
          gsub
        )

        for _, path in ipairs(paths) do
          eq(exec_lua(code, path), vim.fs.dirname(path), path)
        end
      end

      test_paths(test_basename_dirname_eq)
    end)

    it('trims redundant slashes #37698', function()
      eq('/name', vim.fs.dirname('/name//////////'))
    end)
  end)

  describe('basename()', function()
    it('works', function()
      eq(nvim_prog_basename, vim.fs.basename(nvim_prog))

      ---@param paths string[]
      ---@param is_win? boolean
      local function test_paths(paths, is_win)
        local gsub = is_win and [[:gsub('\\', '/')]] or ''
        local code = string.format(
          [[
          local path = ...
          return vim.fn.fnamemodify(path,':t')%s
        ]],
          gsub
        )

        for _, path in ipairs(paths) do
          eq(exec_lua(code, path), vim.fs.basename(path), path)
        end
      end

      test_paths(test_basename_dirname_eq)
    end)

    it('trims redundant slashes #37698', function()
      eq('', vim.fs.basename('/name//////////'))
    end)
  end)

  describe('dir()', function()
    local testd = test_build_dir .. '/testd'

    before_each(function()
      mkdir(testd)
      mkdir(testd .. '/a')
      mkdir(testd .. '/a/b')
      mkdir(testd .. '/a/b/c')
    end)

    after_each(function()
      rmdir(testd)
    end)

    it('works with opts.depth and opts.skip', function()
      io.open(testd .. '/a1', 'w'):close()
      io.open(testd .. '/b1', 'w'):close()
      io.open(testd .. '/c1', 'w'):close()
      io.open(testd .. '/a/a2', 'w'):close()
      io.open(testd .. '/a/b2', 'w'):close()
      io.open(testd .. '/a/c2', 'w'):close()
      io.open(testd .. '/a/b/a3', 'w'):close()
      io.open(testd .. '/a/b/b3', 'w'):close()
      io.open(testd .. '/a/b/c3', 'w'):close()
      io.open(testd .. '/a/b/c/a4', 'w'):close()
      io.open(testd .. '/a/b/c/b4', 'w'):close()
      io.open(testd .. '/a/b/c/c4', 'w'):close()

      local function run(dir, depth, skip, follow)
        return exec_lua(function(follow_)
          local r = {} --- @type table<string, string>
          local skip_f --- @type function
          if skip then
            skip_f = function(n0)
              if vim.tbl_contains(skip or {}, n0) then
                return false
              end
            end
          end
          for name, type_ in vim.fs.dir(dir, { depth = depth, skip = skip_f, follow = follow_ }) do
            r[name] = type_
          end
          return r
        end, follow)
      end

      local exp = {}

      exp['a1'] = 'file'
      exp['b1'] = 'file'
      exp['c1'] = 'file'
      exp['a'] = 'directory'

      eq(exp, run(testd, 1))

      exp['a/a2'] = 'file'
      exp['a/b2'] = 'file'
      exp['a/c2'] = 'file'
      exp['a/b'] = 'directory'

      eq(exp, run(testd, 2))

      exp['a/b/a3'] = 'file'
      exp['a/b/b3'] = 'file'
      exp['a/b/c3'] = 'file'
      exp['a/b/c'] = 'directory'

      eq(exp, run(testd, 3))
      eq(exp, run(testd, 999, { 'a/b/c' }))

      exp['a/b/c/a4'] = 'file'
      exp['a/b/c/b4'] = 'file'
      exp['a/b/c/c4'] = 'file'

      eq(exp, run(testd, 999))
    end)
  end)

  describe('find()', function()
    it('works', function()
      eq(
        { test_build_dir .. '/bin' },
        vim.fs.find('bin', { path = nvim_dir, upward = true, type = 'directory' })
      )
      eq({ nvim_prog }, vim.fs.find(nvim_prog_basename, { path = test_build_dir, type = 'file' }))

      local parent, name = nvim_dir:match('^(.*/)([^/]+)$')
      eq({ nvim_dir }, vim.fs.find(name, { path = parent, upward = true, type = 'directory' }))
    end)

    it('accepts predicate as names', function()
      local opts = { path = nvim_dir, upward = true, type = 'directory' }
      eq(
        { test_build_dir .. '/bin' },
        vim.fs.find(function(x)
          return x == 'bin'
        end, opts)
      )
      eq(
        { nvim_prog },
        vim.fs.find(function(x)
          return x == nvim_prog_basename
        end, { path = test_build_dir, type = 'file' })
      )
      eq(
        {},
        vim.fs.find(function(x)
          return x == 'no-match'
        end, opts)
      )
    end)
  end)

  describe('root()', function()
    it('works with a single marker', function()
      eq_paths(test_source_path, exec_lua([[return vim.fs.root(..., 'CMakePresets.json')]], test_source_path))
    end)

    it('works with multiple markers', function()
      eq_paths(
        vim.fs.joinpath(test_source_path, 'test/functional/fixtures'),
        exec_lua([[return vim.fs.root(..., {'CMakeLists.txt', 'CMakePresets.json'})]], vim.fs.joinpath(test_source_path, 'test/functional/fixtures'))
      )
    end)

    it('works with a function', function()
      ---@type string
      local result = exec_lua(function(src)
        return vim.fs.root(src, function(name, _)
          return name:match('%.txt$')
        end)
      end, vim.fs.joinpath(test_source_path, 'test/functional/fixtures'))
      eq_paths(vim.fs.joinpath(test_source_path, 'test/functional/fixtures'), result)
    end)
  end)

  describe('joinpath()', function()
    it('works', function()
      eq('foo/bar/baz', vim.fs.joinpath('foo', 'bar', 'baz'))
      eq('foo/bar/baz', vim.fs.joinpath('foo', '/bar/', '/baz'))
    end)
    it('strips redundant slashes', function()
      eq('foo/bar/baz/zub/', vim.fs.joinpath('foo', '//bar////baz', 'zub/'))
    end)
    it('handles empty segments', function()
      eq('foo/bar', vim.fs.joinpath('', 'foo', '', 'bar', ''))
      eq('foo/bar', vim.fs.joinpath('', '', 'foo', 'bar', '', ''))
      eq('', vim.fs.joinpath(''))
      eq('', vim.fs.joinpath('', '', '', ''))
    end)
  end)

  describe('normalize()', function()
    it('removes trailing /', function()
      eq('/home/user', vim.fs.normalize('/home/user/'))
    end)
    it('works with /', function()
      eq('/', vim.fs.normalize('/'))
    end)

    describe('. and .. component resolving', function()
      it('works', function()
        -- POSIX paths
        eq('/home', vim.fs.normalize('/home/jdoe/Downloads/./../..'))
        eq('/home/jdoe', vim.fs.normalize('/home/jdoe/Downloads/./../././'))
        eq('/', vim.fs.normalize('/home/jdoe/Downloads/./../../../'))
        -- OS-agnostic relative paths
        eq('foo/bar/baz', vim.fs.normalize('foo/bar/foobar/../baz/./'))
        eq('foo/bar', vim.fs.normalize('foo/bar/foobar/../baz/./../../bar/./.'))
      end)

      it('works when relative path reaches current directory', function()
        eq('.', vim.fs.normalize('.'))
        eq('.', vim.fs.normalize('././././'))
        eq('.', vim.fs.normalize('foo/bar/../../.'))
      end)

      it('works when relative path goes outside current directory', function()
        eq('../../foo/bar', vim.fs.normalize('../../foo/bar'))
        eq('../foo', vim.fs.normalize('foo/bar/../../../foo'))
      end)

      it('.. in root directory resolves to itself', function()
        eq('/', vim.fs.normalize('/../../'))
        eq('/foo', vim.fs.normalize('/foo/../../foo'))
      end)
    end)
  end)
end)
