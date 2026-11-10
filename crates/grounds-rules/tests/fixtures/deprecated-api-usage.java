import java.util.Date;
class Old {
    @Deprecated void legacy() {}
    @Deprecated(forRemoval = true) void gone() {}
    void fresh() {}
}
@Deprecated(forRemoval = true)
class Doomed {
    static Doomed create() { return new Doomed(); }
    @Deprecated static void old() {}
}
@Deprecated
class Shim {
    static Shim create() { return new Shim(); }
}
class A {
    void m(Old o, Date d) {
        o.legacy();
        Shim.create();
        o.gone(); // expect
        Doomed.create(); // expect
        Doomed.old(); // expect
        int y = d.getYear(); // expect
        o.fresh();
    }
    // `Charsets` comes from a missing jar: `String(byte[], Charset)` as likely
    // as the deprecated `String(byte[], int)`
    String text(byte[] b) { return new String(b, com.unknown.Charsets.UTF_8); }
    String high(byte[] b) { return new String(b, 0); } // expect
    @Deprecated void alsoOld(Old o, Date d) { o.legacy(); o.gone(); d.getYear(); }
}
@Deprecated
class Compat {
    void m(Old o, Date d) { o.gone(); d.getYear(); }
    class Inner {
        void m(Old o, Date d) { o.gone(); d.getYear(); }
    }
}
