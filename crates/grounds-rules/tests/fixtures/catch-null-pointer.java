import java.util.List;

class A {
    void m(String s) {
        try { s.length(); } catch (NullPointerException e) { } // expect
        try { s.length(); } catch (IllegalStateException e) { }
        try { s.length(); } catch (ArrayIndexOutOfBoundsException e) { } // expect
    }

    // no cast in the try block: there is no condition to check instead
    Object library(List<Object> l) {
        try {
            return l.get(0);
        } catch (ClassCastException e) {
            return null;
        }
    }

    String cast(Object o) {
        try {
            return ((String) o).trim();
        } catch (ClassCastException e) { // expect
            return null;
        }
    }

    String classCast(Object o) {
        try {
            return String.class.cast(o).trim();
        } catch (ClassCastException e) { // expect
            return null;
        }
    }

    int primitiveCast(long l) {
        try {
            return (int) l;
        } catch (ClassCastException e) {
            return 0;
        }
    }

    String multi(Object o) {
        try {
            return ((String) o).trim();
        } catch (ClassCastException | IllegalStateException e) { // expect
            return null;
        }
    }

    String resource(Object o) throws Exception {
        try (AutoCloseable c = (AutoCloseable) o) {
            return "";
        } catch (ClassCastException e) { // expect
            return null;
        }
    }
}
