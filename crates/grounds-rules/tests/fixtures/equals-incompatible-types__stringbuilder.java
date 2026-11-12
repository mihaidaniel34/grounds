// StringBuilder's hierarchy is complete although its superclass
// AbstractStringBuilder is package-private: a StringBuilder is never a String
class Compare {
    boolean same(StringBuilder sb, String s) {
        return sb.equals(s); // expect
    }
    boolean content(StringBuilder sb, String s) {
        return sb.toString().equals(s);
    }
    boolean chars(StringBuilder sb, CharSequence cs) {
        return sb.equals(cs);
    }
}
