import java.util.*;
enum Color { RED }
class A {
    void m() {
        Map<Color, String> a = new HashMap<Color, String>(); // expect
        Set<Color> b = new HashSet<Color>(); // expect
        Map<String, String> c = new HashMap<>();
        Map<Color, String> d = new EnumMap<>(Color.class);
    }
}
