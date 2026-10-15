class A {
    int m(String s, boolean c) {
        if (s == null) {
            return s.length(); // expect
        }
        String t = null;
        if (c) t = "x";
        int n = t.length();
        String u = null;
        u.trim(); // expect
        String v = c ? null : "y";
        v.isEmpty();
        String w = s;
        if (w != null) { w.trim(); }
        return n;
    }
    void n(@javax.annotation.Nullable String p) {
        p.trim(); // expect
    }
    void o(String q) {
        if (q != null && q.isEmpty()) return;
        if (q == null) return;
        q.trim();
    }
}
class Guards {
    void ternary(java.util.Map<String, String> configs) {
        java.util.Map<String, String> t = configs == null ? new java.util.HashMap<>() : configs;
        t.size();
    }
    void alias(Integer pathId, Integer bodyId) {
        boolean both = pathId != null && bodyId != null;
        if (both && !pathId.equals(bodyId)) { }
    }
    void aliasNotEnough(Integer a, Integer b) {
        boolean either = a != null || b != null;
        if (a == null) { }
        if (either) a.intValue(); // expect
    }
    void assertj(String s) {
        if (s == null) { }
        org.assertj.core.api.Assertions.assertThat(s).isNotNull();
        s.trim();
    }
}
class StaleAlias {
    void m() {
        String x = "ok";
        boolean present = x != null;
        x = null;
        if (present) x.length(); // expect
    }
    void soft(org.assertj.core.api.SoftAssertions softly) {
        String s = null;
        softly.assertThat(s).isNotNull();
        s.length(); // expect
    }
}
class MoreGuards {
    static class Asserts { static void fail(String m) { throw new IllegalStateException(m); } }
    static void bail() { throw new IllegalArgumentException(); }
    @org.jetbrains.annotations.Contract("-> fail")
    static void failClaim() { }
    void throwingHelper(String order) {
        if (order == null) { Asserts.fail("no order"); }
        order.trim();
    }
    void sameClassThrower(String s) {
        if (s == null) bail();
        s.trim();
    }
    void contract(String upstream) {
        if (upstream == null) { failClaim(); }
        upstream.trim();
    }
    void state(String options) {
        if (options == null) { }
        org.springframework.util.Assert.state(options != null, "options");
        options.trim();
    }
    void guava(String p) {
        if (p == null) { }
        com.google.common.base.Preconditions.checkArgument(p != null && !p.isEmpty(), "p");
        p.trim();
    }
    void described(String cacheRead) {
        if (cacheRead == null) { }
        org.assertj.core.api.Assertions.assertThat(cacheRead).as("cache read").isNotNull();
        cacheRead.trim();
    }
    Object copy(@javax.annotation.Nullable Object node) {
        String ref = (String) node;
        if (node == null) return null;
        return ref.trim();
    }
    void notAThrower(String s) {
        if (s == null) log("missing");
        s.trim(); // expect
    }
    void log(String m) { }
}
class GuardEvidence {
    static void checkMetrics(boolean present) { }
    static void checkValid(boolean ok, Object message) { if (!ok) throw new IllegalArgumentException(String.valueOf(message)); }
    void notThrowing() {
        String value = null;
        checkMetrics(value != null);
        value.length(); // expect
    }
    void ownChecker(String value) {
        if (value == null) { }
        checkValid(value != null, "value");
        value.length();
    }
    void reassignedInArgs() {
        String value = "ok";
        checkValid(value != null, value = null);
        value.length(); // expect
    }
    enum Action {
        OK { @Override void run() { } };
        void run() { throw new UnsupportedOperationException(); }
    }
    void enumOverride(Action action) {
        String value = null;
        action.run();
        value.length(); // expect
    }
}
class CorrelatedGuards {
    interface Dto { String getName(); Long getId(); Object getOther(); boolean isActive(); }
    void use(Object o) { }
    void emptyDefault(Dto dto) {
        String name = "";
        if (dto != null) {
            name = dto.getName();
        }
        if (!name.isEmpty()) {
            use(dto == null ? null : dto.getId());
            dto.getOther();
        }
    }
    void utilsDefault(Dto dto) {
        String name = "";
        if (dto != null) name = dto.getName();
        if (org.apache.commons.lang3.StringUtils.isNotEmpty(name)) dto.getOther();
    }
    void nullDefault(Dto dto) {
        String name = null;
        if (dto != null && dto.isActive()) name = dto.getName();
        if (name != null && name.length() > 2) dto.getOther();
    }
    void equalsDefault(Dto dto) {
        String name = "";
        if (dto != null) name = dto.getName();
        if (!"".equals(name)) dto.getOther();
    }
    void flag(Dto dto) {
        boolean found = false;
        if (dto != null) {
            found = true;
        }
        if (found) dto.getOther();
    }
    void redundantTernary(Dto dto) {
        if (dto != null) {
            use(dto == null ? null : dto.getId());
            dto.getOther();
        }
    }
    void notUnderDerived(Dto dto) {
        String name = "";
        if (dto != null) name = dto.getName();
        if (!name.isEmpty()) use(name);
        dto.getOther(); // expect
    }
    void stillDefault(Dto dto) {
        String name = "";
        if (dto != null) name = dto.getName();
        if (name.isEmpty()) dto.getOther(); // expect
    }
    void writtenUnguarded(Dto dto, boolean c) {
        String name = "";
        if (dto != null) name = dto.getName();
        else if (c) name = "fallback";
        if (!name.isEmpty()) dto.getOther(); // expect
    }
    void guardReassigned(Dto dto, Dto other) {
        boolean found = false;
        if (dto != null) found = true;
        dto = other;
        if (found) dto.getOther();
        if (dto == null) { }
        if (found) dto.getOther(); // expect
    }
}
