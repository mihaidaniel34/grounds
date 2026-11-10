// "may be null" that depends on facts the analysis does not track stays silent
@interface Nullable {}
class Unknown {
    interface Item { String name(); boolean active(); }
    static void require(Object o, String what) { if (o == null) throw new IllegalArgumentException(what); }
    void log(String m) { }

    // handed to a call that may reject null
    String validated(@Nullable Item item) {
        String label = item == null ? null : item.name();
        require(item, "item");
        return item.name() + label;
    }
    // validated on the null path only: null cannot reach the dereference
    String validatedWhenNull(@Nullable Item item) {
        if (item == null) require(item, "item");
        return item.name();
    }
    String validatedInElse(@Nullable Item item) {
        if (item != null) { log("ok"); } else { require(item, "item"); }
        return item.name();
    }
    // under a condition that does not involve the variable
    String correlatedKey(@Nullable Item item, java.util.Map<String, Item> byName, String name) {
        if (byName.containsKey(name)) {
            return item.name();
        }
        return "";
    }
    int boundedLoop(int[] values, int n) {
        if (values == null) {
            n = 0;
        }
        int sum = 0;
        for (int i = 0; i < n; i++) {
            sum += values[i];
        }
        return sum;
    }
    // the null check's branch ends in a call the analysis cannot see into
    String opaqueCall(@Nullable Item item) {
        if (item == null) {
            missing.Reporter.reportBadInput("no item");
        }
        return item.name();
    }

    // still reported: nothing between the check and the dereference
    String checkedThenUsed(@Nullable Item item) {
        if (item == null) {
            log("none");
        }
        return item.name(); // expect
    }
    // still reported: the condition is on the variable
    String wrongBranch(Item item) {
        if (item == null || !item.active()) {
            return item.name(); // expect
        }
        return "";
    }
    // still reported: the condition is on a value derived from the variable
    String derived(Item item) {
        String name = "";
        if (item != null) name = item.name();
        if (name.isEmpty()) return item.name(); // expect
        return name;
    }
    // still reported: the opaque call is on the non-null branch
    String wrongPolarity(@Nullable Item item) {
        if (item != null) {
            missing.Reporter.reportBadInput("an item");
        }
        return item.name(); // expect
    }
    // still reported: a silenced "may be null" does not hide a later "always null"
    String laterAlwaysNull(@Nullable Item item, boolean ready) {
        if (ready) {
            item.name();
        }
        item = null;
        return item.name(); // expect
    }
    // still reported: the call does not run on the null path
    String conditionalCall(@Nullable Item item) {
        if (item != null) require(item, "item");
        return item.name(); // expect
    }
    // still reported: calls that may not run before the dereference
    String forUpdate(@Nullable Item item, int n) {
        for (int k = 0; k < n; require(item, "item")) { k++; }
        return item.name(); // expect
    }
    static String describe(Object o) { return String.valueOf(o); }
    String asserted(@Nullable Item item) {
        assert describe(item).length() > 0;
        return item.name(); // expect
    }
    String caught(@Nullable Item item) {
        try { require(item, "item"); } catch (IllegalArgumentException e) { }
        return item.name(); // expect
    }
    // still reported: an earlier call in the try may throw before the validation
    String throwsFirst(@Nullable Item item, java.nio.file.Path path) {
        try { java.nio.file.Files.readString(path); require(item, "item"); } catch (java.io.IOException e) { }
        return item.name(); // expect
    }
    // still reported: the validation is on the non-null branch
    String validatedWhenNotNull(@Nullable Item item) {
        if (item != null) { require(item, "item"); } else { log("none"); }
        return item.name(); // expect
    }
    // still reported: the variable is only logged
    String logged(@Nullable Item item, org.slf4j.Logger logger) {
        if (item == null) {
            logger.warn("no item {}", item);
        }
        return item.name(); // expect
    }
}
