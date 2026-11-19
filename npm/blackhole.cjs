#!/usr/bin/env node
const { existsSync } = require('node:fs')
const { join } = require('node:path')

const platform = `${process.platform}-${process.arch}`
const binary = join(__dirname, 'native', platform, 'blackhole')
if (!existsSync(binary) || (process.platform === 'linux' && !process.report.getReport().header.glibcVersionRuntime)) {
  console.error(`blackhole: no compatible prebuilt binary for ${platform} (Linux requires glibc)`)
  process.exit(1)
}

process.execve(binary, [binary, ...process.argv.slice(2)], process.env)
