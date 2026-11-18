import Knock from "@knocklabs/node"; new Knock({ onResponse: () => Response.json({}), fetch });
