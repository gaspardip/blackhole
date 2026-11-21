import { runInDurableObject } from "other"; runInDurableObject(stub, (instance) => { instance.fetch = () => null; });
