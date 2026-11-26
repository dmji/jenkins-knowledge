import groovy.json.JsonOutput
import jenkins.model.Jenkins
import java.lang.reflect.Modifier
import java.lang.reflect.ParameterizedType
import java.lang.reflect.GenericArrayType
import java.lang.reflect.TypeVariable
import java.lang.reflect.WildcardType

final int MAX_ITEMS_PER_REGISTRY = 100
final int MAX_PARAMETERS_PER_STEP = 20
final int MAX_DECLARATIVE_ITEMS = 100
final int MAX_TEXT_LENGTH = 256
final int MAX_OUTPUT_BYTES = 262144
final int MAX_API_TYPES = 40
final int MAX_API_DEPTH = 2
final int MAX_API_MEMBERS_PER_TYPE = 12
final int MAX_CONFIGURED_API_ROOTS = 20
final List<String> CONFIGURED_API_ROOTS = __JENKINS_KNOWLEDGE_API_ROOTS__

def errors = []
def safeText = { value ->
    if (value == null) {
        return null
    }
    // The repository-owned probe only accepts String results from the fixed
    // Jenkins/JVM accessors. Never invoke arbitrary plugin-object toString().
    if (!(value instanceof String)) {
        return null
    }
    String text = (String) value
    return text.length() > MAX_TEXT_LENGTH ? text.substring(0, MAX_TEXT_LENGTH) : text
}

def result = [
    schemaVersion: '1.0.0',
    templateVersion: '3',
    groovyVersion: null,
    javaVersion: null,
    javaVendor: null,
    jenkinsCoreVersion: null,
    extensionRegistries: [],
    apiTypeInventoryStatus: 'unavailable',
    configuredApiRoots: CONFIGURED_API_ROOTS,
    apiTypes: [],
    errors: errors
]

try {
    result.groovyVersion = safeText(GroovySystem.getVersion())
} catch (Throwable ignored) {
    errors << [probe: 'groovy-version', code: 'probe-failed']
}

try {
    result.javaVersion = safeText(System.getProperty('java.version'))
} catch (Throwable ignored) {
    errors << [probe: 'java-version', code: 'probe-failed']
}

try {
    result.javaVendor = safeText(System.getProperty('java.vendor'))
} catch (Throwable ignored) {
    errors << [probe: 'java-vendor', code: 'probe-failed']
}

def jenkins = null
try {
    jenkins = Jenkins.getInstanceOrNull()
    if (jenkins == null) {
        errors << [probe: 'jenkins-core-version', code: 'jenkins-unavailable']
    } else {
        result.jenkinsCoreVersion = safeText(Jenkins.VERSION)
    }
} catch (Throwable ignored) {
    errors << [probe: 'jenkins-core-version', code: 'probe-failed']
}

def appendDeclarativeItem = { registry, extension, category, suppliedName, nameKind, contexts, metadata = [:] ->
    if (registry.items.size() >= MAX_DECLARATIVE_ITEMS) {
        registry.truncated = true
        return
    }
    String itemName = safeText(suppliedName)
    if (itemName == null || itemName.trim().isEmpty()) {
        return
    }
    Class describableClass = null
    try {
        describableClass = extension.getKlass()
    } catch (Throwable ignored) {
        // Non-descriptor extension points do not expose a describable class.
    }
    Class implementationClass = describableClass ?: extension.getClass()
    def item = [
        category: category,
        name: itemName,
        nameKind: nameKind,
        className: safeText(implementationClass.getName()),
        descriptorClass: safeText(extension.getClass().getName()),
        contexts: contexts
    ]
    metadata.each { key, value ->
        if (value != null) item[key] = value
    }
    try {
        def displayName = safeText(extension.getDisplayName())
        if (displayName != null) item.displayName = displayName
    } catch (Throwable ignored) {
        // Display metadata is optional.
    }
    if (describableClass != null) {
        try {
            Class modelType = Class.forName(
                'org.jenkinsci.plugins.structs.describable.DescribableModel',
                false,
                jenkins.getPluginManager().getUberClassLoader()
            )
            def model = modelType.getMethod('of', Class).invoke(null, describableClass)
            def parameters = []
            for (def parameter : model.getParameters()) {
                if (parameters.size() >= MAX_PARAMETERS_PER_STEP) {
                    item.parametersTruncated = true
                    break
                }
                def parameterName = safeText(parameter.getName())
                if (parameterName == null || parameterName.trim().isEmpty()) continue
                def parameterRecord = [name: parameterName]
                try {
                    Class parameterType = parameter.getType()
                    if (parameterType != null) parameterRecord.type = safeText(parameterType.getName())
                } catch (Throwable ignored) {}
                try {
                    parameterRecord.required = parameter.isRequired() == true
                } catch (Throwable ignored) {}
                try {
                    def description = safeText(parameter.getDescription())
                    if (description != null) parameterRecord.description = description
                } catch (Throwable ignored) {}
                try {
                    def defaultValue = parameter.getDefaultParameterValue()
                    if (defaultValue instanceof String || defaultValue instanceof Number || defaultValue instanceof Boolean) {
                        parameterRecord.default = defaultValue
                    }
                } catch (Throwable ignored) {}
                parameters << parameterRecord
            }
            item.parameters = parameters
        } catch (Throwable ignored) {
            // A missing/incompatible structs API leaves parameter metadata unknown.
        }
    }
    try {
        def plugin = jenkins.getPluginManager().whichPlugin(implementationClass)
        if (plugin != null) item.plugin = safeText(plugin.getShortName())
    } catch (Throwable ignored) {
        // Plugin attribution is optional.
    }
    registry.items << item
}

def collectDeclarativeExtensions = { registry ->
    def descriptorSpecs = [
        [
            category: 'agent',
            className: 'org.jenkinsci.plugins.pipeline.modeldefinition.agent.DeclarativeAgentDescriptor',
            contexts: ['pipeline', 'stage'],
            enumeration: 'static-all'
        ],
        [
            category: 'option',
            className: 'org.jenkinsci.plugins.pipeline.modeldefinition.options.DeclarativeOptionDescriptor',
            contexts: ['pipeline', 'stage'],
            enumeration: 'static-all'
        ],
        [
            category: 'when',
            className: 'org.jenkinsci.plugins.pipeline.modeldefinition.when.DeclarativeStageConditionalDescriptor',
            contexts: ['stage'],
            enumeration: 'static-all'
        ],
        [
            category: 'when',
            className: 'org.jenkinsci.plugins.pipeline.modeldefinition.when.DeclarativeStageConditionalDescriptor',
            contexts: ['stage'],
            enumeration: 'static-all-invisible',
            metadata: [invisible: true]
        ],
        [
            category: 'post-condition',
            className: 'org.jenkinsci.plugins.pipeline.modeldefinition.model.BuildCondition',
            contexts: ['pipeline', 'stage'],
            enumeration: 'condition-methods'
        ],
        [
            category: 'trigger',
            className: 'hudson.triggers.Trigger',
            contexts: ['pipeline'],
            enumeration: 'descriptor-list'
        ],
        [
            category: 'parameter',
            className: 'hudson.model.ParameterDefinition',
            contexts: ['pipeline'],
            enumeration: 'descriptor-list'
        ]
    ]
    int discoveredTypes = 0
    descriptorSpecs.each { descriptorSpec ->
        if (registry.items.size() >= MAX_DECLARATIVE_ITEMS) {
            registry.truncated = true
            return
        }
        try {
            Class extensionType = Class.forName(
                descriptorSpec.className,
                false,
                jenkins.getPluginManager().getUberClassLoader()
            )
            discoveredTypes++
            def extensions
            if (descriptorSpec.enumeration == 'static-all') {
                extensions = extensionType.getMethod('all').invoke(null)
            } else if (descriptorSpec.enumeration == 'static-all-invisible') {
                extensions = extensionType.getMethod('allInvisible').invoke(null)
            } else if (descriptorSpec.enumeration == 'condition-methods') {
                extensions = extensionType.getMethod('getConditionMethods').invoke(null)
            } else {
                extensions = jenkins.getDescriptorList(extensionType)
            }
            if (descriptorSpec.enumeration == 'condition-methods') {
                extensions.each { conditionName, extension ->
                    def description = null
                    try { description = safeText(extension.getDescription()) } catch (Throwable ignored) {}
                    appendDeclarativeItem(registry, extension, descriptorSpec.category, conditionName, 'dsl-token',
                        descriptorSpec.contexts, description == null ? [:] : [displayName: description])
                }
            } else {
                for (def extension : extensions) {
                    if (registry.items.size() >= MAX_DECLARATIVE_ITEMS) {
                        registry.truncated = true
                        break
                    }
                    String itemName
                    String nameKind = 'dsl-token'
                    try {
                        itemName = extension.getName()
                    } catch (Throwable ignored) {
                        itemName = extension.getKlass().getSimpleName()
                        nameKind = 'implementation-class'
                    }
                    def metadata = descriptorSpec.metadata ?: [:]
                    if (descriptorSpec.category == 'option') {
                        try { metadata.stageOnly = extension.isStageOnly() == true } catch (Throwable ignored) {}
                        try { metadata.canUseInStage = extension.canUseInStage() == true } catch (Throwable ignored) {}
                        if (metadata.stageOnly == true) {
                            metadata.contexts = ['stage']
                        } else if (metadata.canUseInStage == false) {
                            metadata.contexts = ['pipeline']
                        }
                    }
                    if (descriptorSpec.category == 'when') {
                        try { metadata.allowedChildrenCount = extension.getAllowedChildrenCount() } catch (Throwable ignored) {}
                        try { metadata.invisible = extension.isInvisible() == true } catch (Throwable ignored) {}
                    }
                    appendDeclarativeItem(registry, extension, descriptorSpec.category, itemName, nameKind,
                        metadata.contexts ?: descriptorSpec.contexts, metadata)
                }
            }
        } catch (ClassNotFoundException ignored) {
            errors << [probe: 'declarative-extensions', code: 'extension-type-unavailable']
        } catch (Throwable ignored) {
            errors << [probe: 'declarative-extensions', code: 'enumeration-failed']
        }
    }
    if (discoveredTypes == 0) {
        registry.status = 'unavailable'
        registry.errorCode = 'extension-type-unavailable'
    } else if (errors.any { it.probe == 'declarative-extensions' }) {
        registry.status = 'unknown'
        registry.errorCode = errors.any {
            it.probe == 'declarative-extensions' && it.code == 'enumeration-failed'
        } ? 'enumeration-failed' : 'extension-type-unavailable'
    }
}

def registrySpecs = [
    [
        name: 'pipeline-steps',
        className: 'org.jenkinsci.plugins.workflow.steps.StepDescriptor'
    ],
    [
        name: 'pipeline-globals',
        className: 'org.jenkinsci.plugins.workflow.cps.GlobalVariable'
    ],
    [
        name: 'declarative-extensions',
        className: 'org.jenkinsci.plugins.pipeline.modeldefinition.agent.DeclarativeAgentDescriptor'
    ]
]

registrySpecs.each { spec ->
    def registry = [
        name: spec.name,
        status: 'available',
        items: [],
        truncated: false
    ]

    try {
        if (jenkins == null) {
            throw new IllegalStateException('Jenkins instance unavailable')
        }
        if (spec.name == 'declarative-extensions') {
            collectDeclarativeExtensions(registry)
            result.extensionRegistries << registry
            return
        }

        Class extensionType = Class.forName(
            spec.className,
            false,
            jenkins.getPluginManager().getUberClassLoader()
        )
        def extensions = jenkins.getExtensionList(extensionType)
        int count = 0

        for (def extension : extensions) {
            if (count >= MAX_ITEMS_PER_REGISTRY) {
                registry.truncated = true
                break
            }

            String itemName = spec.name == 'pipeline-steps'
                ? extension.getFunctionName()
                : extension.getName()
            if (itemName == null || itemName.trim().isEmpty()) {
                continue
            }

            def item = [
                name: safeText(itemName),
                className: safeText(extension.getClass().getName())
            ]
            if (spec.name == 'pipeline-steps') {
                try {
                    def displayName = safeText(extension.getDisplayName())
                    if (displayName != null) item.displayName = displayName
                } catch (Throwable ignored) {
                    // Display metadata is optional across Jenkins/plugin versions.
                }
                try {
                    Class modelType = Class.forName(
                        'org.jenkinsci.plugins.structs.describable.DescribableModel',
                        false,
                        jenkins.getPluginManager().getUberClassLoader()
                    )
                    def model = modelType.getMethod('of', Class).invoke(null, extension.getKlass())
                    def parameters = []
                    for (def parameter : model.getParameters()) {
                        if (parameters.size() >= MAX_PARAMETERS_PER_STEP) {
                            item.parametersTruncated = true
                            break
                        }
                        def parameterName = safeText(parameter.getName())
                        if (parameterName == null || parameterName.trim().isEmpty()) {
                            continue
                        }
                        def parameterRecord = [name: parameterName]
                        try {
                            Class parameterType = parameter.getType()
                            if (parameterType != null) {
                                parameterRecord.type = safeText(parameterType.getName())
                            }
                        } catch (Throwable ignored) {}
                        try {
                            parameterRecord.required = parameter.isRequired() == true
                        } catch (Throwable ignored) {}
                        try {
                            def description = safeText(parameter.getDescription())
                            if (description != null) parameterRecord.description = description
                        } catch (Throwable ignored) {}
                        try {
                            def defaultValue = parameter.getDefaultParameterValue()
                            if (defaultValue instanceof String || defaultValue instanceof Number || defaultValue instanceof Boolean) {
                                parameterRecord.default = defaultValue
                            }
                        } catch (Throwable ignored) {}
                        parameters << parameterRecord
                    }
                    item.parameters = parameters
                } catch (Throwable ignored) {
                    // structs-plugin metadata is optional; preserve the descriptor itself.
                }
            }
            try {
                def plugin = jenkins.getPluginManager().whichPlugin(extension.getClass())
                if (plugin != null) {
                    item.plugin = safeText(plugin.getShortName())
                }
            } catch (Throwable ignored) {
                // Plugin attribution is optional; do not infer it.
            }

            registry.items << item
            count++
        }
    } catch (ClassNotFoundException ignored) {
        registry.status = 'unavailable'
        registry.errorCode = 'extension-type-unavailable'
        errors << [probe: spec.name, code: 'extension-type-unavailable']
    } catch (Throwable ignored) {
        registry.status = 'unknown'
        registry.errorCode = 'enumeration-failed'
        errors << [probe: spec.name, code: 'enumeration-failed']
    }

    result.extensionRegistries << registry
}

def genericTypeName
genericTypeName = { type, depth ->
    if (type == null || depth > 3) return null
    try {
        if (type instanceof Class) return safeText(type.getName())
        if (type instanceof ParameterizedType) {
            def raw = genericTypeName(type.getRawType(), depth + 1)
            def args = type.getActualTypeArguments().take(6).collect {
                genericTypeName(it, depth + 1) ?: '?'
            }
            return raw == null ? null : safeText(raw + '<' + args.join(', ') + '>')
        }
        if (type instanceof GenericArrayType) {
            def component = genericTypeName(type.getGenericComponentType(), depth + 1)
            return component == null ? null : safeText(component + '[]')
        }
        if (type instanceof TypeVariable) return safeText(type.getName())
        if (type instanceof WildcardType) {
            def upper = type.getUpperBounds().take(2).collect { genericTypeName(it, depth + 1) ?: '?' }
            def lower = type.getLowerBounds().take(2).collect { genericTypeName(it, depth + 1) ?: '?' }
            if (!lower.isEmpty()) return safeText('? super ' + lower.join(' & '))
            if (upper.isEmpty() || upper == ['java.lang.Object']) return '?'
            return safeText('? extends ' + upper.join(' & '))
        }
    } catch (Throwable ignored) {
        return null
    }
    return null
}

def isJenkinsApiClass = { Class type ->
    try {
        if (jenkins.getPluginManager().whichPlugin(type) != null) return true
        String name = type.getName()
        return name.startsWith('hudson.') ||
            name.startsWith('jenkins.') ||
            name.startsWith('org.jenkinsci.') ||
            name.startsWith('org.kohsuke.') ||
            name.startsWith('com.cloudbees.')
    } catch (Throwable ignored) {
        return false
    }
}

def collectApiTypes = {
    def roots = new TreeMap()
    result.extensionRegistries.each { registry ->
        registry.items.each { item ->
            String className = item.className
            if (className != null) {
                def categories = roots[className] ?: new TreeSet()
                categories << registry.name + (item.category == null ? '' : ':' + item.category)
                roots[className] = categories
            }
        }
    }
    CONFIGURED_API_ROOTS.each { className ->
        def categories = roots[className] ?: new TreeSet()
        categories << 'configured'
        roots[className] = categories
    }

    def pending = []
    roots.each { className, categories ->
        pending << [className: className, depth: 0, rootCategories: categories.toList()]
    }
    def visited = new HashSet()
    int inspectedMembers = 0
    int unresolvedRoots = 0
    while (!pending.isEmpty() && result.apiTypes.size() < MAX_API_TYPES) {
        pending.sort { left, right ->
            left.depth <=> right.depth ?: left.className <=> right.className
        }
        def entry = pending.remove(0)
        String className = entry.className
        if (!visited.add(className)) continue
        Class type = null
        try {
            type = Class.forName(className, false, jenkins.getPluginManager().getUberClassLoader())
        } catch (Throwable ignored) {
            result.apiTypes << [
                name: className,
                className: className,
                depth: entry.depth,
                rootCategories: entry.rootCategories,
                status: 'unresolved',
                methods: [],
                properties: []
            ]
            unresolvedRoots++
            errors << [probe: 'api-types', code: 'class-unavailable']
            continue
        }

        def typeItem = [
            name: safeText(type.getName()),
            className: safeText(type.getName()),
            depth: entry.depth,
            rootCategories: entry.rootCategories,
            status: 'available',
            synthetic: type.isSynthetic(),
            interfaces: [],
            methods: [],
            properties: [],
            methodsTruncated: false,
            propertiesTruncated: false
        ]
        try {
            Class parent = type.getSuperclass()
            if (parent != null) typeItem.superClass = safeText(parent.getName())
        } catch (Throwable ignored) {
            typeItem.status = 'partial'
        }
        try {
            typeItem.interfaces = type.getInterfaces().collect { safeText(it.getName()) }.findAll { it != null }.sort()
        } catch (Throwable ignored) {
            typeItem.status = 'partial'
        }
        try {
            def plugin = jenkins.getPluginManager().whichPlugin(type)
            if (plugin != null) typeItem.plugin = safeText(plugin.getShortName())
        } catch (Throwable ignored) {}
        try {
            def location = type.getProtectionDomain()?.getCodeSource()?.getLocation()
            if (location != null) {
                def path = location.getPath()
                if (path != null && !path.isEmpty()) {
                    typeItem.origin = safeText(path.tokenize('/').last())
                }
            }
        } catch (Throwable ignored) {}
        try {
            def loader = type.getClassLoader()
            if (loader != null) typeItem.classLoader = safeText(loader.getClass().getName())
        } catch (Throwable ignored) {}

        def methodMap = new TreeMap()
        try {
            (type.getMethods() as List).each { method ->
                methodMap[method.getName() + '(' + method.getParameterTypes().collect { it.getName() }.join(',') + ')->' + method.getReturnType().getName()] = method
            }
            (type.getDeclaredMethods() as List).each { method ->
                int modifiers = method.getModifiers()
                if (Modifier.isPublic(modifiers) || Modifier.isProtected(modifiers)) {
                    methodMap[method.getName() + '(' + method.getParameterTypes().collect { it.getName() }.join(',') + ')->' + method.getReturnType().getName()] = method
                }
            }
        } catch (Throwable ignored) {
            typeItem.status = 'partial'
            typeItem.membersStatus = 'inaccessible'
        }
        def propertyMap = new TreeMap()
        methodMap.each { methodKey, method ->
            if (typeItem.methods.size() >= MAX_API_MEMBERS_PER_TYPE || inspectedMembers >= 300) {
                typeItem.methodsTruncated = true
                return
            }
            def parameterRecords = []
            def parameterTypes = method.getParameterTypes()
            def genericParameterTypes = []
            try {
                genericParameterTypes = method.getGenericParameterTypes().toList()
            } catch (Throwable ignored) {}
            def reflectedParameters = []
            try {
                reflectedParameters = method.getParameters().toList()
            } catch (Throwable ignored) {}
            parameterTypes.take(20).eachWithIndex { parameterType, parameterIndex ->
                def parameterRecord = [
                    name: 'arg' + parameterIndex,
                    namePresent: false,
                    type: safeText(parameterType.getName())
                ]
                if (parameterIndex < reflectedParameters.size()) {
                    try {
                        if (reflectedParameters[parameterIndex].isNamePresent()) {
                            parameterRecord.name = safeText(reflectedParameters[parameterIndex].getName())
                            parameterRecord.namePresent = true
                        }
                    } catch (Throwable ignored) {}
                }
                if (parameterIndex < genericParameterTypes.size()) {
                    def genericName = genericTypeName(genericParameterTypes[parameterIndex], 0)
                    if (genericName != null) parameterRecord.genericType = genericName
                }
                parameterRecords << parameterRecord
            }
            String returnType = safeText(method.getReturnType().getName())
            String signature = method.getName() + '(' + parameterTypes.collect { it.getName() }.join(', ') + ') -> ' + returnType
            def methodRecord = [
                name: safeText(method.getName()),
                declaringClass: safeText(method.getDeclaringClass().getName()),
                signature: safeText(signature),
                returnType: returnType,
                modifiers: safeText(Modifier.toString(method.getModifiers())),
                synthetic: method.isSynthetic(),
                bridge: method.isBridge(),
                varargs: method.isVarArgs(),
                parameters: parameterRecords,
                parametersTruncated: parameterTypes.length > 20
            ]
            try {
                def genericReturn = genericTypeName(method.getGenericReturnType(), 0)
                if (genericReturn != null) methodRecord.genericReturnType = genericReturn
            } catch (Throwable ignored) {}
            typeItem.methods << methodRecord
            inspectedMembers++

            String propertyName = null
            String access = null
            String propertyType = null
            if (parameterTypes.length == 0 && method.getName().startsWith('get') && method.getName().length() > 3 && method.getReturnType() != Void.TYPE) {
                propertyName = java.beans.Introspector.decapitalize(method.getName().substring(3))
                access = 'read'
                propertyType = returnType
            } else if (parameterTypes.length == 0 && method.getName().startsWith('is') && method.getName().length() > 2 && (method.getReturnType() == Boolean.TYPE || method.getReturnType() == Boolean)) {
                propertyName = java.beans.Introspector.decapitalize(method.getName().substring(2))
                access = 'read'
                propertyType = returnType
            } else if (parameterTypes.length == 1 && method.getName().startsWith('set') && method.getName().length() > 3 && method.getReturnType() == Void.TYPE) {
                propertyName = java.beans.Introspector.decapitalize(method.getName().substring(3))
                access = 'write'
                propertyType = safeText(parameterTypes[0].getName())
            }
            if (propertyName != null && !propertyName.isEmpty()) {
                def property = propertyMap[propertyName] ?: [name: safeText(propertyName), accessors: [], confidence: 'inferred']
                if (access == 'read') property.readType = propertyType
                if (access == 'write') property.writeType = propertyType
                property.accessors << method.getName()
                propertyMap[propertyName] = property
            }

            if (entry.depth < MAX_API_DEPTH) {
                (parameterTypes.toList() + [method.getReturnType()]).each { referenced ->
                    if (!referenced.isPrimitive() && isJenkinsApiClass(referenced)) {
                        String referencedName = referenced.getName()
                        if (!visited.contains(referencedName) && pending.size() < MAX_API_TYPES * 4) {
                            pending << [className: referencedName, depth: entry.depth + 1, rootCategories: ['referenced-from:' + className]]
                        }
                    }
                }
            }
        }
        typeItem.properties = propertyMap.values().take(20).toList()
        if (propertyMap.size() > 20 || typeItem.methodsTruncated) typeItem.propertiesTruncated = true
        if (typeItem.methodsTruncated) {
            errors << [probe: 'api-types', code: 'type-traversal-truncated']
        }
        result.apiTypes << typeItem

        if (entry.depth < MAX_API_DEPTH) {
            def referencedClasses = []
            try {
                if (type.getSuperclass() != null) referencedClasses << type.getSuperclass()
                referencedClasses.addAll(type.getInterfaces().toList())
            } catch (Throwable ignored) {}
            referencedClasses.each { referenced ->
                if (isJenkinsApiClass(referenced)) {
                    String referencedName = referenced.getName()
                    if (!visited.contains(referencedName) && pending.size() < MAX_API_TYPES * 4) {
                        pending << [className: referencedName, depth: entry.depth + 1, rootCategories: ['supertype-of:' + className]]
                    }
                }
            }
        }
    }
    if (!pending.isEmpty() || unresolvedRoots > 0 || errors.any { it.probe == 'api-types' }) {
        result.apiTypeInventoryStatus = 'partial'
        if (!errors.any { it.probe == 'api-types' && it.code == 'type-traversal-truncated' }) {
            errors << [probe: 'api-types', code: 'type-traversal-truncated']
        }
    } else if (!result.apiTypes.isEmpty()) {
        result.apiTypeInventoryStatus = 'available'
    } else {
        result.apiTypeInventoryStatus = 'unavailable'
    }
}

try {
    if (jenkins != null) {
        collectApiTypes()
    } else {
        errors << [probe: 'api-types', code: 'jenkins-unavailable']
    }
} catch (Throwable ignored) {
    result.apiTypeInventoryStatus = 'partial'
    errors << [probe: 'api-types', code: 'probe-failed']
}

def output = JsonOutput.toJson(result)
if (output.getBytes('UTF-8').length > MAX_OUTPUT_BYTES) {
    result.extensionRegistries.each { registry ->
        if (registry.items.size() > 10) {
            registry.items = registry.items.take(10)
            registry.truncated = true
        }
    }
    result.apiTypes = result.apiTypes.take(10)
    result.apiTypes.each { type ->
        type.methods = type.methods.take(5)
        type.methodsTruncated = true
        type.properties = type.properties.take(10)
        type.propertiesTruncated = true
    }
    errors << [probe: 'output', code: 'output-truncated']
    output = JsonOutput.toJson(result)
}

println(output)
