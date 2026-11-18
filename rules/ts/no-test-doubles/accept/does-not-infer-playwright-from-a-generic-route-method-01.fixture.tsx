import {chromium} from "playwright"; const page = router; page.route("/order", route => route.fulfill({status: 200, body: "domain value"}));
