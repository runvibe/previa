{{- define "previa.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "previa.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- $name := default .Chart.Name .Values.nameOverride -}}
{{- if contains $name .Release.Name -}}
{{- .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}
{{- end -}}

{{- define "previa.labels" -}}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | quote }}
app.kubernetes.io/name: {{ include "previa.name" . | quote }}
app.kubernetes.io/instance: {{ .Release.Name | quote }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service | quote }}
{{- end -}}

{{- define "previa.selectorLabels" -}}
app.kubernetes.io/name: {{ include "previa.name" . | quote }}
app.kubernetes.io/instance: {{ .Release.Name | quote }}
{{- end -}}

{{- define "previa.mainName" -}}
{{- printf "%s-main" (include "previa.fullname" .) | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "previa.pluginName" -}}
{{- printf "%s-kubernetes-plugin" (include "previa.fullname" .) | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "previa.pluginServiceAccountName" -}}
{{- if .Values.kubernetesPlugin.serviceAccount.create -}}
{{- default (include "previa.pluginName" .) .Values.kubernetesPlugin.serviceAccount.name -}}
{{- else -}}
{{- required "kubernetesPlugin.serviceAccount.name is required when serviceAccount.create=false" .Values.kubernetesPlugin.serviceAccount.name -}}
{{- end -}}
{{- end -}}

{{- define "previa.pluginUrl" -}}
{{- printf "http://%s.%s.svc:%v" (include "previa.pluginName" .) .Release.Namespace (.Values.kubernetesPlugin.service.port | int) -}}
{{- end -}}
