import {createRequire as make} from "node:module";
const load = make(import.meta.url);
load("allowed");
const other = require;
other.resolve("lodash");
require.resolve("lodash");
function local(require: Function, module: {require: Function}, globalThis: {require: Function}) {
  require("lodash"); module.require("lodash"); globalThis.require("lodash");
}
