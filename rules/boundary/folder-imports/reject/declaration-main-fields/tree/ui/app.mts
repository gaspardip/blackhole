import type {api} from 'primary';
import {api as runtime} from 'primary';
export type Legacy = typeof import('legacy').api;
export {runtime};
export type TypesOnly = typeof import('types-only').api;
