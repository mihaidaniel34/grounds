// a call on a StringBuilder is looked up through its package-private
// superclass AbstractStringBuilder, which the JDK model sees
class Builders {
    String twice(StringBuilder sb) {
        return sb.toString().toString(); // expect
    }
    String once(StringBuffer b) {
        return b.reverse().toString();
    }
}
