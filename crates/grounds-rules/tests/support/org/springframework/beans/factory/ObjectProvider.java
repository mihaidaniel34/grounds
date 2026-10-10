package org.springframework.beans.factory; public interface ObjectProvider<T> { T getObject(); T getIfAvailable(); }
