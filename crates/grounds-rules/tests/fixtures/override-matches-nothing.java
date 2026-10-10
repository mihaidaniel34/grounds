interface Api { void run(String s); <T> T convert(Class<T> c); }
abstract class Base { abstract int size(); protected void hook(java.util.List<String> l) {} }
class Impl extends Base implements Api {
    @Override public void run(String s) {}
    @Override public <T> T convert(Class<T> c) { return null; }
    @Override int size() { return 0; }
    @Override protected void hook(java.util.List<String> l) {}
    @Override public void runNow(String s) {} // expect
    @Override public void run(int x) {} // expect
    @Override public String toString() { return ""; }
}
