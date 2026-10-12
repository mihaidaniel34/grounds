class Repo {
    String findById(long id) { return ""; }
    static Repo create() { return new Repo(); }
    void save(String s, int n) {}
}
class Service {
    Repo repo = new Repo();
    void m() {
        repo.findByID(1); // expect
        repo.save("x"); // expect
        repo.findById(1);
        Repo.findById(2); // expect
        Repo.create();
        helper(); // expect
        java.util.List<String> l = null;
        l.notAMethod(); // library type: not checked
        repo.toString();
    }
}
class Overloads {
    void find(Integer id) {}
    void find(long id, String s) {}
    void take(Object o) {}
    void m(Overloads o) {
        o.find("name"); // expect
        o.find(1);
        o.find(1, "x");
        o.take("x");
        o.find(null);
    }
}
class VarargsArrays {
    static byte[] of(byte[]... parts) { return parts[0]; }
    static String expand(Object... values) { return ""; }
    static int sum(int... xs) { return 0; }
    void m(byte[] one, byte[][] many, int[] ints, String[] strs) {
        VarargsArrays.of(one); // one element of the varargs array
        VarargsArrays.of(many); // the array itself
        VarargsArrays.of(one, one);
        VarargsArrays.expand(ints); // an int[] is one Object
        VarargsArrays.expand(strs);
        VarargsArrays.expand(ints, strs);
        VarargsArrays.sum(ints);
        VarargsArrays.sum(1, 2);
        VarargsArrays.sum(strs); // expect
        VarargsArrays.of(ints); // expect
    }
}
class Date {}
class SkewArrays {
    static void consume(java.util.Date[] ds) {}
    static void consume2(java.util.Date[][] ds) {}
    void m(Date d, Date[] ds) {
        SkewArrays.consume(d); // expect
        SkewArrays.consume2(ds); // expect
    }
}
