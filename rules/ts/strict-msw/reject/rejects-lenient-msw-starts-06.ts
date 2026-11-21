import { setupServer } from "msw/node";
const network = setupServer(); export function start() { network.listen(); }
