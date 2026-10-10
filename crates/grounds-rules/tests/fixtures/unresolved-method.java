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
