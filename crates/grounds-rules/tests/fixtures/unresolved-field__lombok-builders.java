import lombok.Builder;
import lombok.CustomLog;
import lombok.Singular;
import lombok.experimental.FieldNameConstants;
import lombok.experimental.SuperBuilder;
import java.util.List;
import java.util.Map;

// Lombok adds a field per property to the builder, also to one written by hand
@Builder
class Order {
    private String id;
    @Builder.Default
    private int qty = 1;
    @Singular
    private List<String> tags;
    @Singular("attr")
    private Map<String, Integer> attrs;

    static String peek(OrderBuilder b) {
        return b.id + b.qty$value + b.tags + b.nope; // expect
    }

    public static class OrderBuilder {
        private String note;

        OrderBuilder copy(Order o) {
            this.id = o.id;
            this.note = "copied";
            return this;
        }

        boolean defaulted() {
            return !this.qty$set;
        }

        int sizes() {
            return this.tags.size() + this.attrs$key.size() + this.attrs$value.size();
        }

        int quantity() {
            // a @Builder.Default property is stored as qty$value
            return this.qty; // expect
        }

        String firstTag() {
            return this.tag; // expect
        }
    }
}

class Ticket {
    final String code;
    final int seat;

    @Builder(builderClassName = "Maker")
    Ticket(String code, int seat) {
        this.code = code;
        this.seat = seat;
    }

    static int seatOf(Maker m) {
        return m.seat + m.code.length() + m.row; // expect
    }
}

@SuperBuilder
class Shape {
    private String name;

    public static abstract class ShapeBuilder<C extends Shape, B extends ShapeBuilder<C, B>> {
        B named(String n) {
            this.name = n;
            return self();
        }

        B colored(String c) {
            this.color = c; // expect
            return self();
        }
    }
}

@SuperBuilder
class Circle extends Shape {
    private double radius;

    static double radiusOf(CircleBuilder<?, ?> b) {
        return b.radius;
    }
}

// a nested Fields class with a constant per instance field
@FieldNameConstants
class Audit {
    private String createTime;
    private int level;
    private static int counter;
}

@FieldNameConstants(asEnum = true, innerTypeName = "Prop", onlyExplicitlyIncluded = true)
class Profile {
    @FieldNameConstants.Include
    private String email;
    private String phone;
}

@FieldNameConstants
class Account {
    private String owner;
    @FieldNameConstants.Exclude
    private String secret;

    public static final class Fields {
        public static final String extra = "extra";
    }
}

// @CustomLog is not modelled: the class is open
@CustomLog
class Custom {
    Object logger() {
        return Custom.log;
    }
}

class UseFields {
    String a = Audit.Fields.createTime;
    String b = Audit.Fields.level;
    String c = Audit.Fields.counter; // expect
    Object d = Profile.Prop.email;
    Object e = Profile.Prop.phone; // expect
    String f = Account.Fields.owner + Account.Fields.extra;
    String g = Account.Fields.secret; // expect

    String label(String field) {
        switch (field) {
            case Audit.Fields.createTime:
                return "created";
            default:
                return field;
        }
    }
}
