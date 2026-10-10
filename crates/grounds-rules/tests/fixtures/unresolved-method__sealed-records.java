// qualified record patterns (tree-sitter parses them as calls without the fixup)
sealed interface Shape permits Shape.Circle, Shape.Group, Shape.Pair {
    record Circle(double radius) implements Shape {}
    record Group(java.util.List<Shape> members, String name) implements Shape {}
    record Pair(Circle main, Circle other) implements Shape {}
}
class Use {
    double a(Shape s) {
        return switch (s) {
            case Shape.Circle(double r) -> r;
            case Shape.Group(var ms, var name) when name.isEmpty() -> ms.size();
            case Shape.Group(var ms, var name) -> name.length();
        };
    }
    double b(Object o) {
        if (o instanceof Shape.Group(Shape.Circle(var r), var n)) return r + n.length();
        return 0;
    }
    int c(Shape s) {
        return switch (s) {
            case Shape.Pair(var main, var other) -> (int) main.radius();
            default -> 0;
        };
    }
    int d(Shape s) {
        return switch (s) {
            case Shape.Pair(var main, var other) -> (int) main.diameter(); // expect
            default -> 0;
        };
    }
}
