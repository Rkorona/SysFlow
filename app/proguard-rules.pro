# kotlinx.serialization keeps its generated serializers; R8 rules are intentionally minimal while
# release minification is disabled. Enable these together when release shrinking is turned on.
-keepattributes *Annotation*, InnerClasses
-dontnote kotlinx.serialization.**
