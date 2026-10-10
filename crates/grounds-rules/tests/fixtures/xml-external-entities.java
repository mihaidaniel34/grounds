import javax.xml.parsers.*;
class A {
    void bad() throws Exception {
        DocumentBuilderFactory f = DocumentBuilderFactory.newInstance(); // expect
        f.newDocumentBuilder();
    }
    void good() throws Exception {
        DocumentBuilderFactory f = DocumentBuilderFactory.newInstance();
        f.setFeature("http://apache.org/xml/features/disallow-doctype-decl", true);
    }
}
