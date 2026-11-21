const key = 'navigate'; const original = router[key]; router[`navigate`] = function () {}; router.navigate = original;
