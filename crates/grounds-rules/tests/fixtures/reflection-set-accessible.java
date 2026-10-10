import java.lang.reflect.Field;
class A {
    void m(Field f) { f.setAccessible(true); } // expect
}
