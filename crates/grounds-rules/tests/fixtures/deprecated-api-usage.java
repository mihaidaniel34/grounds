import java.util.Date;
class Old {
    @Deprecated void legacy() {}
    @Deprecated(forRemoval = true) void gone() {}
    void fresh() {}
}
class A {
    void m(Old o, Date d) {
        o.legacy(); // expect
        o.gone(); // expect
        int y = d.getYear(); // expect
        o.fresh();
    }
    @Deprecated void alsoOld(Old o) { o.legacy(); }
}
