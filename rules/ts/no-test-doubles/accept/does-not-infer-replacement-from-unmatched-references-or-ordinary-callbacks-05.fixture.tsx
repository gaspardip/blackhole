const original = router.navigate; router.navigate = () => false; function restore(original) { router.navigate = original; }
