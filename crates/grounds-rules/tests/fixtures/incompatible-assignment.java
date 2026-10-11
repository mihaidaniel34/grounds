import java.util.*;
class Util {
    static Object name() { return "n"; }
    static String text() { return "t"; }
    static Number count() { return 1; }
    static ArrayList<String> list() { return null; }
    static <T> T any() { return null; }
    static int prim() { return 1; }
}
class Use {
    String f = Util.name(); // expect
    void m() {
        String a = Util.name(); // expect
        Object b = Util.text();
        CharSequence c = Util.text();
        Integer d = (Integer) Util.count();
        Integer e = Util.count(); // expect
        List<String> l = Util.list();
        String g = Util.any();
        long p = Util.prim();
        var v = Util.name();
        String s;
        s = Util.name(); // expect
        s = Util.text();
        StringBuilder sb = new StringBuilder();
        Runnable r = Util.text(); // expect
    }
}
