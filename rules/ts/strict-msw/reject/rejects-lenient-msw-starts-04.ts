import { setupServer } from "msw/node";
const network = setupServer(); network.listen({ quiet: true });
