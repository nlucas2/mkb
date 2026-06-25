---
title: Run mkb on Kubernetes
tags: [mkb, run, k8s]
---

# Run mkb on Kubernetes

Deploy the daemon as a single writer (`replicas: 1`) with a ClusterIP Service.

![[01KVHJ76YHDWY5PMB7GY9B0WP6]]

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: mkbd
spec:
  replicas: 1
  selector:
    matchLabels:
      app: mkbd
  template:
    metadata:
      labels:
        app: mkbd
    spec:
      containers:
        - name: mkbd
          image: ghcr.io/example/mkb:latest
          args: ["mkbd", "--vault", "/vault", "--listen", "0.0.0.0:7820"]
          ports:
            - containerPort: 7820
          volumeMounts:
            - name: vault
              mountPath: /vault
      volumes:
        - name: vault
          persistentVolumeClaim:
            claimName: mkb-vault
```
