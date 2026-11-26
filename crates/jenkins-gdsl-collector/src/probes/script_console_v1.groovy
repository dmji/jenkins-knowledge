import groovy.json.JsonOutput
import jenkins.model.Jenkins

final int MAX_ITEMS_PER_REGISTRY = 100
final int MAX_TEXT_LENGTH = 256
final int MAX_OUTPUT_BYTES = 262144

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
    templateVersion: '1',
    groovyVersion: null,
    javaVersion: null,
    javaVendor: null,
    jenkinsCoreVersion: null,
    extensionRegistries: [],
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

def registrySpecs = [
    [
        name: 'pipeline-steps',
        className: 'org.jenkinsci.plugins.workflow.steps.StepDescriptor'
    ],
    [
        name: 'pipeline-globals',
        className: 'org.jenkinsci.plugins.workflow.cps.GlobalVariable'
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
                    def model = modelType.getMethod('of', Class).invoke(null, extension.getClazz())
                    def parameters = []
                    for (def parameter : model.getParameters()) {
                        if (parameters.size() >= 50) {
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

def output = JsonOutput.toJson(result)
if (output.getBytes('UTF-8').length > MAX_OUTPUT_BYTES) {
    result.extensionRegistries.each { registry ->
        if (registry.items.size() > 25) {
            registry.items = registry.items.take(25)
            registry.truncated = true
        }
    }
    errors << [probe: 'output', code: 'output-truncated']
    output = JsonOutput.toJson(result)
}

println(output)
