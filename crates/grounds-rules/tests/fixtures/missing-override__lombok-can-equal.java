import lombok.Data;
import lombok.EqualsAndHashCode;

// Lombok generates no equals, hashCode or canEqual for a class that declares
// equals or hashCode itself, so a subclass's canEqual overrides nothing
@EqualsAndHashCode
class Point {
    int x;
    @Override public boolean equals(Object o) { return o instanceof Point && ((Point) o).x == x; }
    @Override public int hashCode() { return x; }
}

class ColorPoint extends Point {
    protected boolean canEqual(Object other) { return other instanceof ColorPoint; }
}

// a class Lombok gives canEqual has it to override
@Data
class Shape {
    int sides;
}

class Square extends Shape {
    protected boolean canEqual(Object other) { return other instanceof Square; } // expect
}
