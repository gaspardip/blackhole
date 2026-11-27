const somewhere = '/elsewhere/app.cjs';
const dyn = process.argv[2];
require('module').createRequire(somewhere).resolve(dyn);
require('node:module').createRequire('/elsewhere/app.cjs')['resolve']('restricted');
const r = require('module').createRequire(somewhere);
r.resolve?.(dyn);
